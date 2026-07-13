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
  selector、unlimited widow/orphan lookahead、Fragmentation L3 準拠の
  flex/grid multi-page 対応)
- **per-page で PageBox が変わる混合サイズ PDF を native 対応**
- **2 cursor モデル + probe layout**: DOM cursor と Emission cursor を分離し、
  LayoutBuffer 内で probe layout を保持する構造 (Finding #2 対応)
- **ReflowPolicy trait による dirty tracking の拡張余地確保**: M1〜M8 は
  AggressiveCommit のみ実装、DirtyDeferred / FullReflow は Future Work
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
- **Dirty tracking の実装** (M1〜M8): ReflowPolicy trait は raikiri-traits に
  定義するが、`DirtyDeferred` / `FullReflow` 実装は post-M8 Future Work。
  M1〜M8 では `AggressiveCommit` のみ (probe 限界到達で即 fallback commit)
- **External re-render 用 invalidation infra**: internal LayoutBuffer の
  dirty tracking (Future Work) とは別次元、raikiri は Consumer からの
  mutation → re-render の flow を持たない

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

blitz workspace 構成に対応させ、`stylo` 相当の位置に `raikiri-style` を置く 10
crate 構成 (dev/compat 含めて 13)。GCPM は cascade 側 (raikiri-style) と layout
側 (raikiri-dom) に自然分割し、独立 crate は持たない。

```
raikiri-spike/
├── Cargo.toml (workspace)
├── crates/
│   ├── raikiri-traits/       # 共有 trait 定義 + 中立モデル型 (blitz-traits 相当)
│   │                            RenderSink, NetworkProvider, ReplacedResolver,
│   │                            LookaheadPolicy, TargetResolver, EmissionPolicy,
│   │                            ReflowPolicy,
│   │                            Dom/Element/Node,
│   │                            PageFragment, PageBox, PageContext,
│   │                            LayoutBuffer, TargetRegistry, GcpmDirective,
│   │                            LookaheadConfig, TargetConvergence, BatchConfig
│   ├── raikiri-style/        # ★ stylo 相当: CSS engine
│   │                            - cssparser + selectors integration
│   │                            - 統一 RuleTree (通常 rule + @page/counter/
│   │                              string-set/running/target-* at-rule)
│   │                            - cascade → ComputedValues
│   │                            - GCPM directive extraction (cascade 副産物)
│   │                            - @page rule 解決
│   ├── raikiri-dom/          # ★ blitz-dom 相当: DOM data + layout engine
│   │                            - DOM data model (Node/Element/Attribute)
│   │                            - taffy 統合 (block/flex/grid layout)
│   │                            - parley 統合 (text shaping)
│   │                            - PageStream + LayoutBuffer + Strategy 実装
│   │                              (BoundedLookahead/UnboundedLookahead,
│   │                               PlaceholderTargetResolver/RegistryTargetResolver,
│   │                               ImmediateEmission/DeferredEmission,
│   │                               AggressiveCommit)
│   │                            - GCPM runtime (PageContext state 管理、
│   │                              TargetRegistry, directive 適用)
│   │                            - PageBoxCache
│   │                            - render_with() 低レベル driver
│   ├── raikiri-html/         # 薄い parser (blitz-html 相当)
│   │                            html5ever wrapper + RaikiriTreeSink
│   │                            Consumer が wrap して sanitize/inject/rewrite
│   │                            parse 結果を raikiri-dom Document へ流し込む
│   ├── raikiri-paint/        # PageFragment → anyrender::PaintScene walker
│   │                            (VRT や debug output 用途、blitz-paint 相当)
│   ├── raikiri-net/          # NoOpProvider (default) / SandboxedProvider (future)
│   │                            (blitz-net 相当)
│   ├── raikiri/              # umbrella crate: primary Consumer API
│   │                            全 sub-crate re-export + render_streaming/
│   │                            render_batch orchestrator + html_to_png helper
│   ├── raikiri-blitz-compat/ # blitz 互換 layer (M6 で追加)
│   │                            fulgur migration 支援、型 shape 互換
│   ├── raikiri-wpt/          # dev-only: WPT runner + blitz oracle
│   └── raikiri-vrt/          # dev-only: anyrender_vello_cpu wrapper + PNG diff
├── wpt/                       # W3C web-platform-tests (submodule)
├── expectations/              # blitz-baseline.txt + raikiri-current.txt
├── docs/superpowers/specs/    # 本 doc を含む
└── examples/
```

### 4.0 GCPM の 2 side 分割

blitz には GCPM がないため、raikiri は仕様レベルの natural な境界で 2 side に
分割する:

| GCPM 側面 | 帰属 | 内容 |
|---|---|---|
| **static side** (cascade 副産物) | **raikiri-style** | @page rule 解析、`counter-increment`/`counter-reset` を GcpmDirective::CounterIncrement/Reset として emit、`string-set` を GcpmDirective::StringSet として emit、`position: running(name)` を RunningTemplate 登録として emit、`content: string()/counter()/target-*()` を ContentValueItem として parse |
| **runtime side** (layout 時 state) | **raikiri-dom** | PageContext (counter tree、named string 4-snapshot、running bindings) の管理、TargetRegistry (target-* placeholder emit と resolve)、directive の application (walk 中に counter increment 等を実行)、@page rule の per-page resolution (page_index + page_name → 実効 PageBox) |
| **shared types** | **raikiri-traits** | GcpmDirective, ContentValueItem, PageContext, TargetRegistry, PageBox, PageFragment |

この分割で、GCPM を独立 crate にしなくても Blitz backport 時に「blitz-style
に static side を追加、blitz-dom に runtime side を追加、blitz-traits に
GCPM 型を追加」で提案可能。

### 依存 DAG

blitz と同じ形の DAG:

```
raikiri-traits (foundation)
   ▲
   │
   ├── raikiri-net
   │
   ├── raikiri-style (stylo 相当、GCPM static side)
   │       ▲
   │       │
   │       └── raikiri-dom (blitz-dom 相当、layout + GCPM runtime side)
   │               ▲
   │               │
   │               ├── raikiri-html (thin parser、dom を通じて Document 構築)
   │               │
   │               └── raikiri-paint (dom の computed style と layout 結果を読む)
   │                        ▲
   │                        │
   │                        └── raikiri (umbrella)
   │                                ▲
   │                                │
   │                                ├── raikiri-blitz-compat (M6)
   │                                ├── raikiri-vrt (dev)
   │                                └── raikiri-wpt (dev)
```

**blitz との対応**:
- raikiri-traits ↔ blitz-traits
- raikiri-style ↔ stylo (Servo 由来の外部 dep 相当、raikiri では workspace 内)
- raikiri-dom ↔ blitz-dom (stylo を使う、layout を含む)
- raikiri-html ↔ blitz-html (薄い parser)
- raikiri-paint ↔ blitz-paint
- raikiri-net ↔ blitz-net
- raikiri ↔ blitz (umbrella)

相互依存なし。DAG。cycle なし。逆方向依存なし。

### 依存 / 型帰属テーブル (Finding #3 対応)

reviewer 提案の明示表：

| 型 / 関数 | 定義 crate | 主要な使用 crate |
|---|---|---|
| Sink / Provider / Resolver trait (`RenderSink`, `NetworkProvider`, `ReplacedResolver`) | raikiri-traits | Consumer (impl), raikiri-dom (呼出) |
| Strategy trait (`LookaheadPolicy`, `TargetResolver`, `EmissionPolicy`, `ReflowPolicy`) | raikiri-traits | raikiri-dom (impl + 呼出), Consumer (advanced impl) |
| `Dom` / `Element` / `Node` (trait) | raikiri-traits | 全 crate |
| `PageFragment`, `PageBox`, `PageContext`, `TargetRegistry`, `LayoutBuffer`, `GcpmDirective`, `ContentValueItem` (中立モデル型) | raikiri-traits | raikiri-style (emit), raikiri-dom (use), raikiri-paint (read), Consumer |
| `LookaheadConfig`, `TargetConvergence`, `BatchConfig`, `ContainerOverflowFallback`, `ReflowAction`, `DirtyDeadline` | raikiri-traits | Consumer |
| `RaikiriTreeSink`, `UncascadedDocument` | raikiri-html | raikiri (umbrella, orchestrator), Consumer (wrap) |
| `parse()`, `iter_replaced_elements()` | raikiri-html | raikiri (umbrella) |
| `RuleTree`, `ComputedValues` | raikiri-style | raikiri-dom (使用), raikiri (advanced re-export) |
| `cascade()` | raikiri-style | raikiri (umbrella) |
| GCPM static extraction (directive emit) | raikiri-style | raikiri (umbrella 経由で raikiri-dom へ) |
| `Document` (fully cascaded、layout 前) | raikiri-dom | raikiri (umbrella), Consumer (advanced) |
| Layout runtime (taffy 統合, parley 統合, PageStream, LayoutBuffer 実装) | raikiri-dom | raikiri (umbrella の render_with) |
| Strategy 実装 (BoundedLookahead, UnboundedLookahead, PlaceholderTargetResolver, RegistryTargetResolver, ImmediateEmission, DeferredEmission, AggressiveCommit) | raikiri-dom | raikiri (umbrella 経由), Consumer (advanced) |
| GCPM runtime (PageContext state 管理, TargetRegistry 実装) | raikiri-dom | raikiri (umbrella) |
| `PageBoxCache` (crate-private) | raikiri-dom | raikiri-dom 内部のみ |
| `render_with()` (低レベル driver) | raikiri-dom | raikiri (umbrella 経由), Consumer (advanced) |
| `paint_to_scene()` | raikiri-paint | raikiri-vrt, Consumer |
| `NoOpNetworkProvider`, (future) `SandboxedNetworkProvider` | raikiri-net | Consumer |
| `parse_html()`, `render_streaming()`, `render_batch()`, `html_to_png()` | raikiri (umbrella) | Consumer |

**Consumer 目線**: `use raikiri::*` だけで足りる。sub-crate 直接依存も可能
(advanced case)。

**circular なし**: raikiri-html は raikiri-dom に依存するが cascade を呼ばず、
umbrella crate が orchestrator として raikiri-style::cascade() を呼び出す形。
raikiri-dom は raikiri-style を使うだけで、cascade を呼び返さない。

### crate 責務詳細

#### `raikiri-traits`

foundation crate。他 crate が共有する trait / 中立モデル型を集約。実装は持たず、
他 crate への依存も最小。

```rust
// ── Sink / Provider / Resolver ────────────────────────────────
pub trait RenderSink: Send {
    /// 1 ページ確定次第呼ばれる (Streaming: 逐次、Batch: 全 layout 完了後まとめて)
    fn accept_page(&mut self, page: PageFragment) -> std::io::Result<()>;

    /// 全 accept_page 完了後、render() が呼ぶ最終通知。
    /// summary で TargetRegistry 最終状態を Consumer に届け、Consumer は
    /// 未解決 slot を patch する機会を得る (§4.x completion protocol)。
    /// Consumer 側の resource 解放 (PDF trailer 書出等) は本 method の責務外で、
    /// Consumer が自分で管理する (`sink.finalize_pdf()` 等を別途呼ぶ)。
    fn finish_render(&mut self, summary: RenderSummary) -> std::io::Result<()>;
}

// ── Completion protocol 型 (Finding #4 対応) ────────────────────
pub struct RenderSummary {
    pub total_pages: u32,
    pub target_registry: TargetRegistry,        // 完全な resolved 状態
    pub unresolved_targets: Vec<UnresolvedTarget>,
    pub emitted_target_slots: Vec<EmittedSlotInfo>,  // 全ページで発行した slot 一覧
}

pub struct UnresolvedTarget {
    pub slot_id: TargetSlotId,
    pub fragment_id: Symbol,
    pub reason: UnresolvedReason,
}

pub enum UnresolvedReason {
    /// fragment id がどこにも定義されていない
    NotFound,
    /// NPassConverge が max_iterations で収束せず
    ConvergenceFailed,
    /// Consumer 側 policy でエラー扱い
    ConsumerRejected,
}

pub struct EmittedSlotInfo {
    pub slot_id: TargetSlotId,
    pub fragment_id: Symbol,
    pub kind: TargetKind,
}

/// slot の一意識別子 (Consumer が patch table の key に使う)
/// (page_index, sequence) は decode 順で unique、byte-identical 保証あり
pub struct TargetSlotId {
    pub page_index: u32,
    pub sequence: u32,   // ページ内での通し番号 (0-indexed、target-* 出現順)
}

pub trait ReplacedResolver {
    fn resolve(&self, req: ResolverRequest<'_>) -> Result<IntrinsicBox, ResolverError>;
}

pub trait NetworkProvider {
    fn fetch(&self, url: &Url) -> Result<Bytes, NetworkError>;
}

// ── Strategy traits (Streaming と Batch で変わる差分だけ) ───────
/// LayoutBuffer の lookahead 幅を制御
pub trait LookaheadPolicy {
    fn max_widow_orphan_lines(&self) -> Option<usize>;              // None = unbounded
    fn max_break_avoid_subtree_blocks(&self) -> Option<usize>;
    /// flex/grid container の probe layout 上限 (Finding #2 対応)
    /// None = unbounded (Batch: container 全体を Fragmentation L3 準拠に layout)
    /// Some(N) = N ページ相当まで、超えたら ReflowPolicy に委譲
    fn max_container_probe_pages(&self) -> Option<usize>;
    fn allow_cross_size_lookahead(&self) -> bool;
}

/// target-* の解決方式 (placeholder emit / 事前 registry lookup)
pub trait TargetResolver {
    fn resolve(&mut self, req: TargetRequest<'_>, ctx: &PageContext) -> ResolvedTarget;
}

/// PageFragment の emit タイミング (immediate / deferred)
pub trait EmissionPolicy {
    fn emit(&mut self, page: PageFragment, sink: &mut dyn RenderSink)
        -> std::io::Result<()>;
    fn finish(&mut self, sink: &mut dyn RenderSink) -> std::io::Result<()>;
}

/// probe 限界到達時の挙動、および dirty tracking の余地
/// M1〜M8 は AggressiveCommit のみ実装、DirtyDeferred/FullReflow は Future Work
pub trait ReflowPolicy {
    /// probe 限界到達時に "即 fallback commit" するか "dirty flag で defer" するか
    fn on_probe_limit(&self, ctx: &ProbeContext) -> ReflowAction;
    /// dirty tracking を使う場合の memory 上限
    fn max_dirty_entries(&self) -> Option<usize>;
}

pub enum ReflowAction {
    /// 即 fallback で commit、取り消し不可 (Streaming preset default)
    CommitWithFallback(ContainerOverflowFallback),
    /// dirty flag で defer、後続情報で reflow (post-M8 Future Work)
    DeferAsDirty { deadline: DirtyDeadline },
}

pub enum ContainerOverflowFallback {
    ForceBreakBefore,      // 次ページに強制配置 (推奨)
    SimpleFragmentation,   // align-content 等を無視した単純分割
    OverflowClipping,      // 現ページに詰めて overflow
    Error,                 // fail-loud
}

pub enum DirtyDeadline {
    NextPageBoundary,      // 次のページ確定まで defer
    NextContainerStart,    // 次の container 出現まで defer
    DocumentEnd,           // document 末尾まで defer (Batch preset で活用)
}

// ── DOM 抽象 ───────────────────────────────────────────────────
pub trait Dom { /* ... */ }
pub trait Element<'a> { /* ... */ }
pub trait Node<'a> { /* ... */ }

// ── 中立モデル型 (crate 境界を跨いで参照される) ────────────────
pub struct PageFragment { /* ... §11.2 参照 */ }
pub struct PageBox      { /* ... §9    参照 */ }
pub struct PageContext  { /* ... §7.2  参照 */ }
pub struct LayoutBuffer { /* ... §5    参照 */ }
pub struct TargetRegistry { /* ... §7.2 参照 */ }
pub enum   GcpmDirective { /* ... §7.1  参照 */ }
pub struct LookaheadConfig {
    pub widow_line_buffer: usize,
    pub orphan_line_buffer: usize,
    pub break_avoid_max_subtree_blocks: usize,
    pub allow_cross_size_lookahead: bool,
}

pub enum TargetConvergence {
    TwoPass,
    NPassConverge { max_iterations: u32 },
}
```

**`DocumentPass` / `DomTransform` は含めない** — Consumer は
`RaikiriTreeSink` を wrap する pattern を使用 (5.2 節参照)。

**中立モデル型の帰属**: `PageFragment` / `PageBox` / `PageContext` /
`LayoutBuffer` / `TargetRegistry` / `GcpmDirective` は複数 crate の境界を跨いで
参照される（strategy trait の入出力、sink の入力、cascade の出力など）ため、
foundation crate として raikiri-traits に置く。実装ロジックは各機能 crate 側
(raikiri-style の cascade、raikiri-dom の layout runtime) に残し、型定義だけを
foundational 化する。

#### `raikiri-style` (stylo 相当)

**stylo リプレース crate**。cssparser + selectors を base に、cascade /
specificity / ComputedValues / RuleTree / GCPM static side を自前実装。

- `cssparser` の 1 pass で **通常 rule と @page 系 at-rule を統一 RuleTree に集約**
- selector matching は `selectors` crate に委ねる
- ComputedValues は paged media / GCPM 対応を含む自前型体系
- `content` プロパティは `Vec<ContentValueItem>` として resolved value (実行時解決)
- **GCPM static side** (cascade 副産物として emit):
  - `counter-increment` / `counter-reset` / `counter-set` を GcpmDirective 化
  - `string-set` を GcpmDirective 化
  - `position: running(name)` を RunningTemplate 登録として emit
  - `content: string()/counter()/target-*/element()` を ContentValueItem として parse
  - @page rule の cascade order 解決 (per-page 適用は raikiri-dom 側)

Selectors 対応範囲:
- L3 base、interactive 系 (`:hover`, `:focus`, `:link`, `:visited`, `:target`,
  `:enabled`, `:checked`) は parse は通すが常に non-matching (fail-closed:
  実装しない = match しない)
- L4 (`:is()`, `:where()`, `:not()`) は採用、優先度低
- L4 backward-reference (`:has()`, `:nth-last-child()`, `:blank`) は Phase A で
  full DOM を持つ以上、**mode 非依存で常時対応可能**。実装優先度は低いが、実装
  したら常時有効

#### `raikiri-dom` (blitz-dom 相当)

**DOM + layout engine + GCPM runtime**。blitz-dom の shape で、raikiri-style
を cascade に使い、taffy + parley を layout に使う。

- **DOM data model**: Node / Element / Attribute、`raikiri-traits::Dom` を実装
- **taffy 統合**: block / flex / grid の layout
- **parley 統合**: text shaping、BiDi、font selection
- **PageStream**: crate-private state machine (per-page emit)、DOM cursor と
  Emission cursor を保持
- **LayoutBuffer**: widow/orphan/break-inside/container probe lookahead の
  独立ユニット (中立型は raikiri-traits)
- **Strategy 実装群**: `BoundedLookahead` / `UnboundedLookahead`,
  `PlaceholderTargetResolver` / `RegistryTargetResolver`,
  `ImmediateEmission` / `DeferredEmission`,
  `AggressiveCommit` (M1〜M8 で唯一実装される ReflowPolicy)
- **GCPM runtime side**:
  - PageContext (counter tree、named string 4-snapshot、running bindings) の管理
  - TargetRegistry (target-* placeholder emit と resolve)
  - Directive の application (walk 中に counter increment / string set を実行)
  - @page rule の per-page resolution (page_index + page_name → 実効 PageBox)
- **PageBoxCache**: 同じ (page_name, parity, is_first, is_blank) の @page 解決
  結果を再利用
- **render_with(...)** 低レベル driver (任意 strategy を受け取る、advanced 向け)
- **Per-page 内部の rayon 並列化** (16 margin box slots、paragraph text shape、
  multi-column)

```rust
// 各 strategy の実装 (raikiri-traits の trait を impl)
pub struct BoundedLookahead(pub LookaheadConfig);
pub struct UnboundedLookahead;
pub struct PlaceholderTargetResolver { /* pending_slots */ }
pub struct RegistryTargetResolver    { /* pre-built registry */ }
pub struct ImmediateEmission;
pub struct DeferredEmission          { /* Vec<PageFragment> */ }
pub struct AggressiveCommit          { /* M1〜M8 唯一の ReflowPolicy 実装 */ }

// Advanced driver: strategy を直接指定
pub fn render_with<L, T, E, R>(
    input: impl std::io::Read,
    options: &ParseOptions,
    defaults: PageDefaults,
    resolver: &dyn ReplacedResolver,
    sink: &mut dyn RenderSink,
    lookahead: L,
    target: T,
    emission: E,
    reflow: R,
) -> std::io::Result<()>
where
    L: LookaheadPolicy,
    T: TargetResolver,
    E: EmissionPolicy,
    R: ReflowPolicy;
```

上位の `render_streaming` / `render_batch` は umbrella crate `raikiri` が提供
(§5.3)。

#### `raikiri-html` (blitz-html 相当)

薄い HTML parser wrapper。**責務は parse だけ**、cascade / layout は含まない。
raikiri-dom を通じて Document を構築する。

- html5ever wrapper + `RaikiriTreeSink`
- Consumer が `html5ever::tree_builder::TreeSink` として wrap 可能
  (sanitize / inject / rewrite)
- parse 結果を raikiri-dom の Document に流し込む
- `iter_replaced_elements` で Consumer が replaced element を先読み可能

```rust
pub fn parse<R: std::io::Read>(input: R, options: &ParseOptions) 
    -> Result<UncascadedDocument, ParseError>;

pub fn parse_with_sink<R, S>(input: R, sink: S, options: &ParseOptions) 
    -> Result<UncascadedDocument, ParseError>
where R: std::io::Read, S: html5ever::tree_builder::TreeSink;

pub fn iter_replaced_elements(doc: &UncascadedDocument) 
    -> impl Iterator<Item = ReplacedElementRef<'_>>;

pub struct ParseOptions<'a> {
    pub extra_stylesheets: &'a [&'a str],
    pub network: Option<&'a dyn NetworkProvider>,
    pub base_url: Option<Url>,
}
```

**注意**: `parse` は cascade 前の `UncascadedDocument` を返す。cascade は
raikiri-style で実行、fully cascaded `Document` の組立ては umbrella `raikiri`
が orchestrator として行う (§5.3 参照)。

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

**primary Consumer API**。fulgur は `use raikiri::*` のみで足りる。**orchestrator
としても機能** — parse → cascade → Document 組立てを担う。

```rust
pub use raikiri_html::{parse as raw_parse, ParseOptions, iter_replaced_elements,
                        UncascadedDocument};
pub use raikiri_style::{RuleTree, ComputedValues};  // advanced 用
pub use raikiri_dom::{
    Document, PageDefaults, render_with,
    // strategy 実装 (advanced 向け)
    BoundedLookahead, UnboundedLookahead,
    PlaceholderTargetResolver, RegistryTargetResolver,
    ImmediateEmission, DeferredEmission,
    AggressiveCommit,
};
pub use raikiri_traits::{
    RenderSink, ReplacedResolver, NetworkProvider,
    LookaheadPolicy, TargetResolver, EmissionPolicy, ReflowPolicy,   // strategy trait
    PageFragment, PageBox, PageContext,                              // 中立モデル型
    LookaheadConfig, TargetConvergence, BatchConfig,
    ContainerOverflowFallback, ReflowAction, DirtyDeadline,
};
pub use raikiri_paint;  // sub-module として

/// parse → cascade → Document 組立て (orchestrator)
pub fn parse_html<R: std::io::Read>(input: R, options: &ParseOptions) 
    -> Result<Document, ParseError> {
    let uncascaded = raikiri_html::parse(input, options)?;
    let cascade = raikiri_style::cascade(&uncascaded, options.extra_stylesheets)?;
    Ok(raikiri_dom::Document::assemble(uncascaded, cascade))
}

// ── 通常 Consumer 向け: pre-composed entry ─────────────────
/// Streaming 向け: BoundedLookahead + PlaceholderTargetResolver + ImmediateEmission
pub fn render_streaming(
    input: impl std::io::Read,
    options: &ParseOptions,
    defaults: PageDefaults,
    resolver: &dyn ReplacedResolver,
    lookahead: LookaheadConfig,
    sink: &mut dyn RenderSink,
) -> std::io::Result<()>;

/// Batch 向け: 内部で 2-pass 実行、UnboundedLookahead + RegistryTargetResolver
/// + DeferredEmission を組み合わせ
pub fn render_batch(
    input: impl std::io::Read,
    options: &ParseOptions,
    defaults: PageDefaults,
    resolver: &dyn ReplacedResolver,
    config: BatchConfig,
    sink: &mut dyn RenderSink,
) -> std::io::Result<()>;

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
  【raikiri-html】html5ever chunk streaming → RaikiriTreeSink (Consumer wrap 可)
    │
    ▼
  UncascadedDocument = { dom, stylesheet_sources }  ← raikiri-html の出力
    │
    ▼
  【raikiri-style】cssparser 1-pass:
    - RuleTree (通常 rule + @page 系 at-rule を統一 tree に集約)
    - cascade: selectors::matching + 自前 cascade
    - 出力: Vec<ComputedValues>, Vec<GcpmDirective>, Vec<RunningTemplate>
    │
    ▼
  【raikiri (umbrella) orchestrator】組立て:
    Document = { dom, computed, rule_tree, gcpm_directives, running_templates,
                 seed_page_context }
             ↑ Phase A の produce、Phase B の read-only 入力

Phase B: Page emission (Strategy 群で切替)
─────────────────────────────────────────────
  render_streaming(...)      → BoundedLookahead + PlaceholderTargetResolver
                                + ImmediateEmission + AggressiveCommit
  render_batch(...)          → UnboundedLookahead + RegistryTargetResolver
                                + DeferredEmission + AggressiveCommit
                                (内部で 2-pass、pass 1 で registry 構築、pass 2 で実 render)
  render_with(...)           → 任意の LookaheadPolicy / TargetResolver
                                / EmissionPolicy / ReflowPolicy

  ─── 2 cursor モデル (Finding #2 対応) ────────────────────────────
  DOM cursor    : Phase A で全構築済み Dom を自由に peek ahead
  Emission cursor: PageFragment が emit された時のみ進む
  → 両者のギャップ = look-ahead 幅 (LayoutBuffer + PageContext 差分で保持)

  ─── 内部構造 ────────────────────────────────────────────────────
    LayoutBuffer         (tentative layout state、dirty flag 対応可)
       ↑ policy: LookaheadPolicy (widow/orphan/break-avoid/container probe 上限)
       ↑ reflow: ReflowPolicy (AggressiveCommit / DirtyDeferred / FullReflow)
    TargetResolver       (target-* の解決)
    EmissionPolicy       (emit タイミング + finish)
    PageContext          (counter tree + string 4-snapshot + running + target)
    ResolverPool         (sync resolver、Consumer 責任で並列化)
    PageBoxCache         (同じ (page_name, parity) の @page 解決結果を再利用)

  ─── per page loop (PageBox 先確定 → layout → emit) ──────────────
  ┌ ページ context 決定フェーズ ──────────────────────
  │  1. DOM cursor で次に配置する最初の block を peek (cursor は進めない)
  │  2. block の `page:` property と直前の page_name から新 page_name を決定:
  │        - block.page が current_page_name と異なる → forced break + 新 name
  │        - block.break-before: page → forced break、name は継承
  │        - overflow による自動 break → name は継承
  │        - 最初のページ → block.page (指定なければ default)
  │  3. (page_index, page_name) から @page rule を resolve
  │        (:first / :left / :right / :nth-page(n) / :blank / named)
  │        PageBoxCache で hot path 最適化
  │  4. PageBox を defaults + 実効 @page rule から決定 (available width/height 確定)
  │
  ├ Layout フェーズ (PageBox 確定後、DOM cursor を進めて buffer に取込) ─
  │  5. DOM cursor を進めて次 block を LayoutBuffer に取り込む
  │  6. taffy で block layout / parley で inline layout (available width 使う)
  │       - 通常 block: 収まらなければ break policy 適用
  │       - flex/grid container: max_container_probe_pages 内で probe layout
  │         · probe 限界内で fit → 通常配置
  │         · probe 限界内で spanning → Fragmentation L3 handling
  │         · probe 限界を超え、ReflowPolicy = AggressiveCommit
  │           → ContainerOverflowFallback で決定 (BreakBefore/Simple/Clip)
  │         · probe 限界を超え、ReflowPolicy = DirtyDeferred (post-M8)
  │           → dirty flag で defer、後続の情報で reflow
  │  7. LayoutBuffer.decide_break() で widow/orphan/break-inside 判定
  │  8. PageContext に directive 反映 (counter/string/running/target 更新)
  │  9. LayoutBuffer を PageBox.content.height に収まる分だけ確定
  │
  ├ Margin / emit フェーズ ──────────────────────────
  │ 10. 16 margin box slots layout (rayon 並列可能、PageBox.margins 使う)
  │ 11. PageFragment 組み立てて EmissionPolicy.emit() 呼び出し
  │ 12. Emission cursor を進める、確定領域の layout state を drop
  └ page_index++
```

Fragmentation L3 の scope はページを跨ぐ flex/grid container まで含む。probe
layout で container 全体の subtree を peek し、Fragmentation L3 に沿って各
fragment のサイズを計算。probe 上限を超えた場合の挙動は
`ContainerOverflowFallback` で決定 (§8.2 参照)。

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

- **`CaptionRestructurePass`** → raikiri-dom layout に統合 (native caption-side)
- **`RunningElementPass`** → raikiri-style (extraction) + raikiri-dom (runtime)
- **`StringSetPass`** → raikiri-style (extraction) + raikiri-dom (runtime)
- **`CounterPass`** → raikiri-style cascade + raikiri-dom runtime
- **`InjectCssPass`** → `ParseOptions::extra_stylesheets` に降格 (trait 不要)
- **`BookmarkPass`** → Consumer 側 post-cascade utility (raikiri は heading hint
  のみ提供)

### 5.3 API 面のサマリ

```rust
// fulgur (Consumer) の想定 use
use raikiri::{
    RenderSink, ReplacedResolver, NetworkProvider,
    ParseOptions, PageDefaults, LookaheadConfig, BatchConfig,
    render_streaming, render_batch,
};

let options = ParseOptions {
    extra_stylesheets: &[fulgur_style],
    network: Some(&NoOpNetworkProvider),
    base_url: Some(base),
};

// Streaming (fulgur デフォルト、大規模ドキュメント向け)
raikiri::render_streaming(
    html_input,
    &options,
    PageDefaults::a4(),
    &FulgurResolver::new(font_data, images),
    LookaheadConfig::default(),
    &mut fulgur_pdf_sink,
)?;

// Batch (小〜中規模、target-* 正確化)
raikiri::render_batch(
    html_input,
    &options,
    PageDefaults::a4(),
    &FulgurResolver::new(font_data, images),
    BatchConfig {
        max_document_pages: Some(100),
        target_convergence: TargetConvergence::TwoPass,
    },
    &mut fulgur_pdf_sink,
)?;
```

**Document 型を明示的に使いたい advanced case** (複数モードで試したい、途中で
dump したいなど) には `parse_html()` + `render_with(...)` を提供:

```rust
let doc = raikiri::parse_html(html_input, &options)?;
raikiri::dump_document(&doc)?;  // debug 用

// Renderer を組み替えて試す
raikiri::render_with(
    &doc, PageDefaults::a4(), &resolver, &mut sink_a,
    BoundedLookahead(config), PlaceholderTargetResolver::new(), ImmediateEmission,
)?;
raikiri::render_with(
    &doc, PageDefaults::a4(), &resolver, &mut sink_b,
    UnboundedLookahead, RegistryTargetResolver::from(precomputed), DeferredEmission::new(),
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

## 6. CSS Handling (raikiri-style)

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
- L3 base、interactive 系 (`:hover`, `:focus`, `:link`, `:visited`, `:target`,
  `:enabled`, `:checked`) は selector として parse は通すが、常に non-matching
  として扱う (fail-closed: 実装しない = match しない)
- L4 (`:is()`, `:where()`, `:not()`) 採用、優先度低
- L4 backward-reference (`:has()`, `:nth-last-child()`, `:blank`) は Phase A で
  full DOM を持つ以上、**mode 非依存で常時対応可能**。実装優先度は低いが、
  実装したら常時有効

inline `<style>`:
- **MVP**: `<head>` 内の `<style>` を head 内出現順で登録
- **後の拡張**: body 内 `<style>` を "出現位置以降のみ有効" として採用可能
- **Batch mode でも Streaming mode でも挙動は同一** — mode に依存しない
  (Phase A で全 stylesheet が既に集約されているため)

**重要**: mode (Streaming/Batch) は **layout の pagination policy と target-*
の解決方式にのみ影響し、cascade / selector matching / stylesheet 解釈には
一切影響しない**。

## 7. GCPM IR 明示ノード化 (raikiri-style + raikiri-dom)

### 7.0 GCPM の帰属再確認

blitz には GCPM がないため、raikiri で自前で仕様レベルの natural な境界に
沿って 2 side に分割:

- **static side** (cascade 副産物として生成) → **raikiri-style**
  - `counter-increment` / `counter-reset` / `counter-set` を GcpmDirective 化
  - `string-set` を GcpmDirective 化
  - `position: running(name)` を RunningTemplate 登録として emit
  - `content: string()/counter()/target-*()` を ContentValueItem として parse
  - @page rule の cascade order 解決
- **runtime side** (layout 時 state 管理) → **raikiri-dom**
  - PageContext (counter tree、named string 4-snapshot、running bindings)
  - TargetRegistry (target-* placeholder emit と resolve)
  - Directive の application (walk 中に counter increment 等を実行)
  - @page rule の per-page resolution (page_index + page_name → 実効 PageBox)
- **shared types** → **raikiri-traits**
  - `GcpmDirective`, `ContentValueItem`, `PageContext`, `TargetRegistry`,
    `PageBox`, `PageFragment`, etc.

### 7.1 GcpmDirective

**Producing directive** (raikiri-style の cascade で生成、raikiri-dom の Phase B
walk で PageContext を更新):

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

### 7.4 target-* 解決の 3 戦略

TargetResolver trait の実装として 3 種類を提供:

**`PlaceholderTargetResolver` (Streaming preset)**

- 未解決の target-* に到達したら `TargetSlot { id: TargetSlotId, ... }` を発行
- **`TargetSlotId = (page_index, sequence)`** で stable な識別（Finding #4 対応）
- `ResolvedContent::TargetSlot(slot_id)` として PageFragment.target_slots に emit
- **PageFragment.target_definitions に "このページで定義された fragment id" を並行 emit**
- `RenderSink::finish_render(summary)` で Consumer が最終 `TargetRegistry` を受取り
- Consumer (fulgur → krilla) が Form XObject slot として PDF patch
- streaming 保持

**`RegistryTargetResolver` (Batch preset、TwoPass)**

- Pass 1: 全ページ layout、TargetRegistry 構築、target_definitions 収集
- Pass 2: registry を PageContext に埋め込んで通常の render (TargetSlot は resolved 済み)
- streaming 犠牲

**`ConvergingTargetResolver` (Batch preset、NPassConverge、opt-in)**

- 収束するまで反復。実装は後回し、`max_iterations: 5` 程度で妥協
- 収束せず終了時は `RenderSummary.unresolved_targets` に `ConvergenceFailed` として記録

### 7.5 Consumer patch flow (Finding #4 対応)

Streaming preset での典型的な Consumer flow:

```
per accept_page(page):
  1. page.target_definitions を Consumer 側 definition table に登録
     (fragment_id → (page_index, anchor_position, counter_snapshot, extracted_text))
  2. page.target_slots のうち resolved = None を Consumer 側 pending_patches に登録
     (TargetSlotId → PDF-level slot reference)
  3. page 本体を描画、resolved な TargetSlot は通常描画

finish_render(summary):
  1. summary.emitted_target_slots を iterate
  2. 各 slot について summary.target_registry.resolved から値を取得
     - 取得できたら pending_patches[slot_id] を patch
     - 取得できなければ summary.unresolved_targets を確認、fallback 表示
  3. Consumer 独自の resource close (別 method で呼ぶ、raikiri は関知しない)
```

### 7.6 Slot ID の安定性保証

`TargetSlotId = (page_index, sequence)` の同一性ルール:

- **decode 順で決定**：cascade + layout の walk 順序が deterministic である限り、同じ入力から同じ ID
- **page_index は 0-indexed**、`emit された順` に増加
- **sequence は page 内 0-indexed**、`target-* が evaluate された順`
- byte-identical goal と整合、rayon 並列化されても collect 時に順序を保つ (`IndexedParallelIterator`)

これで Consumer は複数実行で同じ slot ID を key として patch table を保持可能。CI 上での reproducibility も確保。

## 8. Streaming vs Batch (Strategy Pattern)

### 8.1 設計方針

**RenderMode enum は廃止**。Streaming と Batch は「同じ Document を消費する
別々の pipeline」ではなく、「同じ pipeline のうち **3 つの strategy を差し替えた
プリセット**」として実装する。

- Phase A (parse + cascade) は完全に共通。CSS 挙動 (`:has()` を含む selector、
  inline `<style>`、cascade layer など) はどちらの entry でも同一
- Phase B の pagination policy / target-* 解決 / emission タイミングだけを
  strategy trait で分離
- 上位 entry (`render_streaming` / `render_batch`) は「意味のあるプリセットの
  組み合わせ」を提供、`render_with` は任意の組み合わせを許容する低レベル API

### 8.2 差分の一覧 (strategy 化される部分のみ)

| 項目 | Streaming プリセット | Batch プリセット |
|---|---|---|
| **LookaheadPolicy** | `BoundedLookahead(cfg)`: widow/orphan/break-inside/container probe の N line/block/page 上限 | `UnboundedLookahead`: 全 document を buffer |
| **TargetResolver** | `PlaceholderTargetResolver`: placeholder slot 発行、Consumer patch | `RegistryTargetResolver`: 事前構築 registry から lookup |
| **EmissionPolicy** | `ImmediateEmission`: 確定ページを即 sink に渡す | `DeferredEmission`: 全 layout 完了 (+ target 収束) 後にまとめて emit |
| **ReflowPolicy** | `AggressiveCommit`: probe 限界で即 fallback (M1〜M8 default) | `AggressiveCommit` (M1〜M8) / `FullReflow` (post-M8) |
| **target-* 収束** | 収束せず、Consumer が patch | `TwoPass` / `NPassConverge` を config で選択 |
| **container probe** | 上限 N ページ、超えたら `ContainerOverflowFallback` | 上限なし、Fragmentation L3 準拠 |
| **Memory** | `O(DOM + window + 1 page)` | `O(DOM + all pages)` |
| **CSS 挙動** | **Batch と同一** (Phase A で完結) | **Streaming と同一** (Phase A で完結) |

**重要な訂正 (Finding #1 / #7 対応)**: 前版で挙げていた「Streaming では `:has()`
不可、Batch のみ対応」等の差は**存在しない**。それらは Phase A の cascade で
mode 非依存に解決されるため、mode 選択は cascade の挙動に影響しない。

### 8.3 mode 選択 API

```rust
// pre-composed entry (通常 Consumer 向け)
pub fn render_streaming(
    input: impl std::io::Read,
    options: &ParseOptions,
    defaults: PageDefaults,
    resolver: &dyn ReplacedResolver,
    lookahead: LookaheadConfig,
    sink: &mut dyn RenderSink,
) -> std::io::Result<()>;

pub fn render_batch(
    input: impl std::io::Read,
    options: &ParseOptions,
    defaults: PageDefaults,
    resolver: &dyn ReplacedResolver,
    config: BatchConfig,
    sink: &mut dyn RenderSink,
) -> std::io::Result<()>;

// advanced: 任意の strategy 組み合わせ
pub fn render_with<L, T, E, R>(
    doc: &Document,
    defaults: PageDefaults,
    resolver: &dyn ReplacedResolver,
    sink: &mut dyn RenderSink,
    lookahead: L, target: T, emission: E, reflow: R,
) -> std::io::Result<()>
where
    L: LookaheadPolicy,
    T: TargetResolver,
    E: EmissionPolicy,
    R: ReflowPolicy;
```

Consumer (fulgur) は用途で `render_streaming` / `render_batch` を選ぶ。
**RenderSink API は strategy 非依存**、`accept_page` の逐次呼び出しは両者で
同じ shape (ImmediateEmission は即時、DeferredEmission は最後にまとめて呼ぶ)。

### 8.4 fulgur の移行選択肢

- **既存の 2-pass + PDF Form XObject を維持**: raikiri は `render_streaming`
  を使い、`PlaceholderTargetResolver` の `pending_slots` を Consumer が PDF-level
  で patch (fulgur 現行フロー)
- **raikiri の `render_batch` に移行**: fulgur の 2-pass ロジックを削除、
  `RegistryTargetResolver` に委譲
- **カスタム strategy**: `render_with` で fulgur 独自の `FulgurFormXObjectResolver`
  を差し込み、pending_slots を保持しつつ他の strategy はプリセットを流用

## 9. Per-page PageBox

PDF spec は per-page で `/MediaBox` を独立に持てる。CSS Paged Media は
`@page :first`, `@page :left`, `@page landscape-wide` などで per-page 変化を
サポート。

- `render_streaming` / `render_batch` / `render_with` は `PageDefaults` を
  受け取る (単一 PageBox ではない)
- `PageStream` 内部で per-page に @page rule 解決 + `page:` property + defaults
  を cascade
- **`PageBox` は layout の前に確定される** (§5.1 の per page loop の "ページ
  context 決定フェーズ" 参照、Finding #2 対応)
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

### 9.1 page name 遷移ルール (spec 準拠、iterative でない)

**Finding #2 対応**: forced break と named page の相互作用を明示化。

| 状況 | 遷移 |
|---|---|
| 最初のページ | 最初の block の `page:` property を採用 (指定なければ default) |
| ページ N が overflow で自動 break | N+1 は N と同じ page name (継承) |
| 次 block に `page: X` (現在の name と異なる) | forced break-before、N+1 は X |
| 次 block に `page: X` (現在の name と同じ) | 通常継続、break しない |
| 次 block に `break-before: page` | forced break、name は N から継承 |
| 次 block に `break-before: recto/verso` | 空 blank page を 1 枚挿入 (`@page :blank`) して奇偶合わせ、name は継承 |
| 次 block に `break-before: page(X)` | forced break、N+1 は X (CSS Fragmentation L3) |

**iterative でない理由**: 各 block は "自身の page requirement" を明示的に宣言
しており、DOM cursor での先読みで一意に決定できる。iteration が必要になるのは
target-* が page number を変えるケースで、これは `ConvergingTargetResolver`
(Batch preset の option) が扱う別 loop。

### 9.2 PageBoxCache

同じ `(page_name, page_index parity, is_first, is_blank)` の @page rule 解決結果は
再利用可能。`PageBoxCache` は crate-private in raikiri-dom:

```rust
struct PageBoxCache {
    cache: HashMap<PageContextKey, PageBox>,
}
```

`:nth-page(n)` を使う @page rule はページごとに変わるので cache mishit するが、
`:left`/`:right`/named page はキャッシュ効いて layout hot path のオーバーヘッド
最小。

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
    pub target_slots: Vec<TargetSlot>,          // このページで発生した target-* 参照
    pub target_definitions: Vec<TargetDefinition>, // このページで定義された target id (Finding #4)
    pub bookmark_hints: Vec<BookmarkHint>,
    pub link_annotations: Vec<LinkAnnotation>,
    pub heading_structure: Vec<HeadingHint>,
    pub structural_hints: Vec<StructuralHint>,
}

pub struct TargetSlot {
    pub id: TargetSlotId,                    // (page_index, sequence) で安定 (Finding #4)
    pub fragment_id: Symbol,                 // 参照先 URL fragment (例: "chapter-3")
    pub kind: TargetKind,
    pub rect: Rect,                          // 予約領域
    pub resolved: Option<ResolvedTargetValue>, // Streaming: None, Batch: Some
    pub fallback_text: String,               // 未解決時に描画する fallback
}

pub struct TargetDefinition {
    pub fragment_id: Symbol,                 // element の id 属性値
    pub page_index: u32,                     // = このページの index
    pub counter_snapshot: HashMap<Symbol, i32>, // 定義時点の counter 値
    pub extracted_text: Option<String>,      // target-text 用の抽出文字列
    pub anchor_position: Point,              // PDF リンク先座標
}

pub enum TargetKind {
    Counter { name: Symbol, style: CounterStyle },
    Counters { name: Symbol, sep: String, style: CounterStyle },
    Text { part: ContentPart },
    Page,
}

pub enum ResolvedTargetValue {
    Text(String),                            // 描画すべき文字列
    Page(u32),                               // target-page: ページ番号
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
| 座標つき PageFragment 生成 | raikiri-dom |
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
  └─ Streaming / Batch preset-independence test

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

### 12.5 Preset-independence test

**target-* を含まないドキュメント**では `render_streaming` と `render_batch`
の出力 (`PageFragment` 列 + raster) が byte-identical であるべき。cascade / CSS
挙動は preset 非依存なので、target-* が絡まない限り差は生まれないはずで、
これを CI で常時 verify する。

target-* が絡む場合は Streaming が placeholder、Batch が resolved 値を返すので
raster / PDF は必然的に異なる。この場合は「placeholder を fallback 表示に
置換した Streaming 出力」と「Batch 出力」で一致するかを確認する。

これにより strategy 実装間の drift (LookaheadPolicy の変更が意図せず layout を
変えた、EmissionPolicy の順序が壊れた、等) が早期検出できる。

### 12.6 CI 頻度

| テスト | 頻度 | 時間 | 遮断性 |
|---|---|---|---|
| Unit + Integration | PR ごと | 数秒〜数分 | 必須 |
| VRT reftest | PR ごと | 15〜30 分 | 必須 |
| Byte-identical | PR ごと | 数十秒 | 必須 |
| Preset-independence | PR ごと | 数分 | 必須 |
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
    PageDefaults, LookaheadConfig, BatchConfig, TargetConvergence,
    RenderSink, ReplacedResolver, NetworkProvider,
    ParseOptions,
    render_streaming, render_batch,
};

// 1. options を組む (template 展開後の HTML を含む)
let options = ParseOptions {
    extra_stylesheets: &fulgur_stylesheets,
    network: Some(&NoOpNetworkProvider),
    base_url: Some(base),
};

let defaults = PageDefaults::from_cli(cli_size, cli_orientation);
let resolver = FulgurResolver::new(font_data, images);
let mut sink = FulgurPdfSink::new(krilla_doc);

// 2. サイズで render_streaming / render_batch を選ぶ
match estimated_page_count {
    n if n < 100 => {
        raikiri::render_batch(
            html_input, &options, defaults, &resolver,
            BatchConfig {
                max_document_pages: Some(n),
                target_convergence: TargetConvergence::TwoPass,
            },
            &mut sink,
        )?;
    }
    _ => {
        raikiri::render_streaming(
            html_input, &options, defaults, &resolver,
            LookaheadConfig::default(),
            &mut sink,
        )?;
    }
}

// 3. FulgurPdfSink が PageFragment を walk して krilla に落とす
struct FulgurPdfSink {
    krilla_doc: krilla::Document,
    // pending_patches: unresolved TargetSlot を PDF Form XObject ref に mapping
    pending_patches: HashMap<TargetSlotId, PdfFormXObjectRef>,
    definitions: HashMap<Symbol, (u32, Point)>,
}

impl RenderSink for FulgurPdfSink {
    fn accept_page(&mut self, page: PageFragment) -> std::io::Result<()> {
        let mut kr_page = self.krilla_doc.start_page(page.page_box.into())?;
        
        // 通常描画 (略)
        for painted_box in &page.body.boxes { /* draw */ }
        
        // このページで定義された target を Named Destination に登録
        for def in &page.target_definitions {
            self.definitions.insert(def.fragment_id.clone(), (def.page_index, def.anchor_position));
            kr_page.add_named_destination(def.fragment_id.clone(), def.anchor_position)?;
        }
        
        // 未解決 target slot は Form XObject を予約 (fallback を pre-draw)
        for slot in &page.target_slots {
            if slot.resolved.is_none() {
                let form_ref = kr_page.allocate_form_xobject_slot(slot.rect, &slot.fallback_text)?;
                self.pending_patches.insert(slot.id.clone(), form_ref);
            }
            // resolved なら通常描画
        }
        
        kr_page.finish()?;
        Ok(())
    }

    fn finish_render(&mut self, summary: RenderSummary) -> std::io::Result<()> {
        // Finding #4 対応: summary で完全な TargetRegistry を受取り、pending を patch
        for slot_info in &summary.emitted_target_slots {
            if let Some(form_ref) = self.pending_patches.get(&slot_info.slot_id) {
                if let Some(target_info) = summary.target_registry.resolved.get(&slot_info.fragment_id) {
                    let text = format_target_value(target_info, &slot_info.kind);
                    self.krilla_doc.patch_form_xobject(*form_ref, &text)?;
                }
                // 未解決 (summary.unresolved_targets に含まれる) の場合は
                // fallback text は既に pre-draw 済みなのでそのまま
            }
        }
        Ok(())
    }
}

// Consumer 独自の resource close は raikiri の関知外
// (RenderSink::finalize は廃止された)
impl FulgurPdfSink {
    fn finalize_pdf(self) -> std::io::Result<Vec<u8>> {
        // font subset、outline tree 組立て、trailer 書き出し
        self.krilla_doc.into_bytes()
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
- **RenderSink**: Consumer が実装する trait、`accept_page(page)` で per-page
  受取り、`finish_render(summary)` で最終 TargetRegistry を受取り (Finding #4)
- **RenderSummary**: `finish_render` の引数、target_registry / unresolved_targets
  / emitted_target_slots を含む
- **TargetSlotId** `(page_index, sequence)`: slot の安定識別子。byte-identical
  保証あり
- **TargetDefinition**: PageFragment 内、このページで定義された target id 情報
  (fragment_id, page_index, counter_snapshot, extracted_text, anchor_position)
- **LookaheadPolicy** (strategy trait): LayoutBuffer の lookahead 幅を制御。
  Streaming preset は `BoundedLookahead`、Batch preset は `UnboundedLookahead`
- **TargetResolver** (strategy trait): target-* の解決方式。Streaming preset は
  `PlaceholderTargetResolver`、Batch preset は `RegistryTargetResolver`
- **EmissionPolicy** (strategy trait): PageFragment の emit タイミング。
  Streaming preset は `ImmediateEmission`、Batch preset は `DeferredEmission`
- **ReflowPolicy** (strategy trait): probe 限界到達時の挙動。M1〜M8 は
  `AggressiveCommit` のみ実装、`DirtyDeferred` / `FullReflow` は Future Work
- **ContainerOverflowFallback**: probe 限界超過時の fallback 挙動
  (ForceBreakBefore / SimpleFragmentation / OverflowClipping / Error)
- **Streaming preset**: `render_streaming` entry で選ばれる strategy 組み合わせ
- **Batch preset**: `render_batch` entry で選ばれる strategy 組み合わせ
  (内部で 2-pass 実行)
- **DOM cursor / Emission cursor** (Finding #2 対応): 2 cursor モデル。DOM
  cursor は自由に peek ahead、Emission cursor は PageFragment emit 時のみ進む。
  ギャップ = look-ahead 幅
- **PageBoxCache**: 同じ (page_name, parity, is_first, is_blank) の @page 解決
  結果をキャッシュ、layout hot path の最適化
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
- **TargetSlot**: target-* placeholder、Streaming preset で PageFragment に emit
- **RunningTemplate**: `position: running(name)` された subtree の template 保持
