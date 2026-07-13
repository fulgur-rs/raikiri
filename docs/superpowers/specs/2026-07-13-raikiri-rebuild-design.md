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
- **Batch preset を first-class に共存させる**: 小〜中規模ドキュメントで
  以下の M1〜M8 capability を提供 (new review #4 対応):
  - Unbounded lookahead (widow/orphan/break-inside を全 document で判定)
  - `initial_registry` hint 経由の target-* 収束支援 (raikiri 内部 iteration
    はなし、Consumer が chain)
  - Fragmentation L3 準拠の flex/grid multi-page (probe layout で `AggressiveCommit`)
  - `:has()` / `:nth-last-child()` 等の backward selector (実装優先度低)
  - **"full spec compliance" は post-M8 の Future Work** (`FullReflow` +
    dirty tracking で実現予定)
- **per-page で PageBox が変わる混合サイズ PDF を native 対応**
- **2 cursor モデル + probe layout**: DOM cursor と Emission cursor を分離し、
  LayoutBuffer 内で probe layout を保持する構造 (Finding #2 対応)
- **ReflowPolicy trait による dirty tracking の拡張余地確保**: M1〜M8 は
  AggressiveCommit のみ実装、DirtyDeferred / FullReflow は Future Work
- **3 entry point (plan / render_streaming / render_batch)** (Finding #5 対応):
  - `plan` は dry-run、PaintScene/PaintedBox 構築なしの minimal mode
  - Consumer が render 前に document size を検査し、DoS 攻撃的な巨大 document を
    事前拒否可能
  - initial_registry は hint のみ、render は自分で target-* を再計算
- **DoS 耐性の第一級保証**: raikiri は 1 pass 固定、内部 iteration なし。
  Consumer 側で iteration bound を管理
- **同期 ReplacedResolver**: raikiri コアに async runtime を持ち込まない
- **Security 2 層 model** (Finding #6 対応):
  - Layer 1 (DOM sanitize): TreeSink wrap による untrusted HTML 防御
  - Layer 2 (Resource policy): SandboxedNetProvider / SandboxedResolver + 
    ResourcePolicy による fetch 防御
  - 両層を Consumer が実装しないと server-side safety は成立しないことを明示
- **NetworkProvider signature を blitz-traits::NetProvider に shape 一致**:
  Request/AbortSignal/Body/Method を共有 (raikiri は sync return で違い)
- **組版品質と実用性を primary goal** (Finding #9 対応):
  - fulgur が render する代表的 reference documents (契約書、レポート、招待状、
    証明書、目次付き技術書、混合サイズ PDF 等) が期待通りに visual に一致する
    ことが ship 判定基準
  - WPT 準拠は "compliance 基準" ではなく "進捗計測ツール" として扱う
  - カバレッジが低くても実用的なら ship する pragmatic な立場
- **Regression detection の flat rule**: scope 内外を問わず、既 pass →
  新 fail は必ず block。新機能追加が既存の pass を壊すのを防ぐ
- **byte-identical layout output** (決定論): CI reproducibility + regression
  detection のため、reference documents + WPT reftest 範囲で保証
- **WPT progress tracking**: pass rate を nightly で記録、trend として観察。
  絶対値の目標は設定しない。blitz baseline は "少なくとも blitz と同等以上"
  という定性的 signal

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
- **raikiri 内部での target-* 反復収束** (Finding #5 対応): 攻撃者による
  oscillation 誘発を防ぐため、`NPassConverge` / `ConvergingTargetResolver` は
  raikiri 側では非対応。収束が必要な用途は Consumer が `plan` +
  `render_*` を chain して自分の iteration bound で管理
- **無制限メモリ消費**: 各 entry point のメモリ上限を明示、`max_document_pages`
  超過時は fail-fast
- **`std::io::Result<()>` の単純 error 型** (Finding #10 対応): render_* は
  構造化 `RenderError` enum を返し、Consumer が variant で分岐可能に

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

blitz workspace 構成に対応させ、`stylo` 相当の位置に `raikiri-style` を置く。
**Core 7 crate** (traits, style, dom, html, paint, net, raikiri umbrella)
**+ compat 1 crate** (raikiri-blitz-compat、M6 で追加) **+ dev-only 2 crate**
(raikiri-wpt, raikiri-vrt) = **10 crate 総計**。GCPM は cascade 側 (raikiri-style)
と layout 側 (raikiri-dom) に自然分割し、独立 crate は持たない。

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
│   │                            LookaheadConfig, StreamingConfig, BatchConfig,
│   │                            DocumentPlan, PageSummary, TargetDiscrepancy,
│   │                            Request, FetchedResource, Body, Method, AbortSignal,
│   │                            ResourcePolicy, ResourceKind, PolicyViolation
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
│   ├── raikiri-net/          # NoOpNetworkProvider / SandboxedNetProvider<P> +
│   │                            SandboxedResolver<R> + DenyAllPolicy /
│   │                            DefaultSandboxPolicy (blitz-net 相当 + wrapper)
│   ├── raikiri/              # umbrella crate: primary Consumer API
│   │                            全 sub-crate re-export + parse_html / plan /
│   │                            render_streaming / render_batch orchestrator +
│   │                            html_to_png helper
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
| `LookaheadConfig`, `StreamingConfig`, `BatchConfig`, `DocumentPlan`, `PageSummary`, `TargetDiscrepancy`, `ContainerOverflowFallback`, `ReflowAction`, `DirtyDeadline` | raikiri-traits | Consumer |
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
| `NoOpNetworkProvider`, (future) `SandboxedNetProvider` | raikiri-net | Consumer |
| `parse_html()`, `plan()`, `render_streaming()`, `render_batch()`, `html_to_png()` | raikiri (umbrella) | Consumer |

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

// ── Completion protocol 型 (Finding #4 対応、RenderSummary 定義は下の
//     RenderError セクション参照。ここではフィールドの概要のみ)
// RenderSummary { total_pages, target_registry, unresolved_targets,
//                 emitted_target_slots, target_discrepancies, warnings }

pub struct UnresolvedTarget {
    pub slot_id: TargetSlotId,
    pub fragment_id: Symbol,
    pub reason: UnresolvedReason,
}

pub enum UnresolvedReason {
    /// fragment id がどこにも定義されていない
    NotFound,
    /// Consumer 側 policy でエラー扱い
    ConsumerRejected,
    // ConvergenceFailed は削除 (raikiri 内 iteration しないため)
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
    /// req に含まれる URL の scheme/host/size 等の検証は Consumer 責任。
    /// 集中的に policy を効かせたい場合は raikiri-net の SandboxedResolver を wrap
    /// (Finding #6 対応、§10 参照)
    ///
    /// round 4 review #3 対応: Err は常に terminal (RenderError::Resolver)。
    /// Consumer 側 fallback は `Ok(ResolvedIntrinsic { intrinsic, disposition:
    /// Fallback { .. } })` として返し、raikiri は disposition を見て
    /// RenderSummary.warnings に自動記録
    fn resolve(&self, req: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError>;
}

pub struct ResolvedIntrinsic {
    pub intrinsic: IntrinsicBox,
    pub disposition: ResolveDisposition,
}

#[non_exhaustive]
pub enum ResolveDisposition {
    /// 通常の resolve 成功
    Ok,
    /// Consumer が意図的に fallback を選択 (image not found → placeholder 等)
    /// raikiri は WarningKind::ResolverFallback として RenderSummary.warnings に記録
    Fallback { reason: String },
}

/// blitz-traits::NetProvider の shape に揃える (Finding #6 対応)。
/// raikiri は sync core のため callback ではなく sync return。
/// policy 適用は raikiri-net::SandboxedNetProvider による wrap で行う。
pub trait NetworkProvider: Send + Sync {
    fn fetch(&self, request: Request) -> Result<FetchedResource, NetworkError>;
}

pub struct Request {
    pub url: Url,
    pub method: Method,
    pub content_type: Option<String>,
    pub headers: HeaderMap,
    pub body: Body,
    pub signal: Option<AbortSignal>,
    /// raikiri 追加: fetch の目的 (blitz は doc_id だが、raikiri は context 表現)
    pub kind: ResourceKind,
}

pub struct FetchedResource {
    pub bytes: Bytes,
    pub content_type: Option<String>,
    pub final_url: Url,     // redirect 後
    pub encoding: Option<String>,
}

pub enum Body { Bytes(Bytes), Form(FormData), Empty }
pub enum Method { Get, Post, /* ... */ }

/// blitz と同じ AbortSignal (AtomicBool ラッパ)
pub struct AbortSignal(Arc<AtomicBool>);

impl AbortSignal {
    pub fn is_aborted(&self) -> bool { self.0.load(Ordering::Acquire) }
}

pub struct AbortController { pub signal: AbortSignal }
impl AbortController {
    pub fn new() -> Self { /* ... */ }
    pub fn with_timeout(dur: Duration) -> Self { /* thread 立ててtimeoutでabort */ }
    pub fn abort(&self) { /* ... */ }
}

pub enum NetworkError {
    Aborted,
    PolicyViolation(PolicyViolation),
    Io(std::io::Error),
    Http(u16),
    Other(String),
}

// ── ResourcePolicy trait (Finding #6 対応、opt-in) ────────────
pub trait ResourcePolicy: Send + Sync {
    // URL / host 制御
    fn is_scheme_allowed(&self, scheme: &str, kind: ResourceKind) -> bool;
    fn is_host_allowed(&self, host: &str, kind: ResourceKind) -> bool;
    // Redirect 制御
    fn allow_redirect(&self, from: &Url, to: &Url, hop: u32) -> bool;
    fn max_redirect_hops(&self, kind: ResourceKind) -> u32;
    // Size 制限 (byte-level DoS 対策)
    fn max_fetch_bytes(&self, kind: ResourceKind) -> Option<u64>;
    fn max_decoded_bytes(&self, kind: ResourceKind) -> Option<u64>;
    // Timeout (thread hang 対策)
    fn fetch_timeout(&self, kind: ResourceKind) -> Duration;
    fn decode_timeout(&self, kind: ResourceKind) -> Duration;
    // MIME validation
    fn allowed_mime_types(&self, kind: ResourceKind) -> Vec<String>;
    // Recursion (chained @import 対策)
    fn max_import_depth(&self) -> u32;
    fn max_svg_recursion_depth(&self) -> u32;
}

#[non_exhaustive]
pub enum ResourceKind {
    StylesheetImport,    // @import in CSS
    ExternalStylesheet,  // <link rel="stylesheet">
    Image,               // <img src>, background-image
    Font,                // @font-face src
    Svg,                 // external SVG
    MathML,              // external MathML
    Other,
}

pub struct PolicyViolation {
    pub kind: ResourceKind,
    pub url: Url,
    pub violation_type: ViolationType,
    pub details: String,
}

pub enum ViolationType {
    SchemeNotAllowed,
    HostNotAllowed,
    RedirectDenied,
    FetchTooLarge { limit: u64, actual: u64 },
    DecodedTooLarge { limit: u64, actual: u64 },
    Timeout,
    MimeNotAllowed { mime: String },
    RecursionExceeded { depth: u32 },
    Other,
}

// ── RenderError (Finding #10 対応、構造化 error taxonomy) ────────
/// すべての variant は "rendering がそこで停止した" ことを意味する terminal error。
/// Consumer 側 fallback は resolver / network の Consumer 実装内で `Ok(fallback)` を
/// 返すことで表現し、raikiri は fallback が発生したことを `RenderSummary.warnings`
/// で記録する (Finding #1 新review対応: recoverable 分類を廃止)
#[non_exhaustive]
pub enum RenderError {
    /// HTML parse エラー
    Parse(ParseError),
    /// CSS parse / cascade エラー
    Cascade(CascadeError),
    /// Layout エラー
    Layout(LayoutError),
    /// Consumer の resolver が Err を返した
    Resolver(ResolverError),
    /// Consumer の network が Err を返した
    Network(NetworkError),
    /// Resource policy 違反
    Policy(PolicyViolation),
    /// RenderLimits の各種 limit 超過 (fail-fast、round 4 review #1 対応、
    /// 旧 PageLimitExceeded を kind: Pages で吸収)
    LimitExceeded { kind: LimitKind, limit: u64, actual: u64 },
    /// Consumer の sink method (accept_page / finish_render) が Err を返した
    Sink(std::io::Error),
    /// config 不整合 (BatchConfig.initial_registry が不正 等)
    Configuration(String),
    /// std::io::Error 系
    Io(std::io::Error),
}

impl std::error::Error for RenderError { /* ... */ }
impl std::fmt::Display for RenderError { /* ... */ }

#[non_exhaustive]
pub enum LimitKind {
    Pages,
    DomNodes,
    TargetSlots,
    LayoutBufferEntries,
    AggregateBytes,
}

/// AbortSignal による graceful shutdown (partial output あり) を error と別カテゴリで
/// 表現する。render_* は `Result<RenderStatus, RenderError>` を返す
pub enum RenderStatus {
    /// 全ページ emit 完了、finish_render も成功
    Completed(RenderSummary),
    /// AbortSignal による中断。直前まで emit 済み、finish_render は呼ばれない
    Aborted { partial_pages: u32 },
}

pub struct RenderSummary {
    pub total_pages: u32,
    pub target_registry: TargetRegistry,
    pub unresolved_targets: Vec<UnresolvedTarget>,
    pub emitted_target_slots: Vec<EmittedSlotInfo>,
    /// hint と actual の乖離を検知した項目 (Finding #5 対応、Consumer 収束判定用)
    pub target_discrepancies: Vec<TargetDiscrepancy>,
    /// Consumer's fallback usage、policy violation 等の警告
    /// (Finding #1 新review対応: Ok を返した resolver/network の fallback 事象を記録)
    pub warnings: Vec<RenderWarning>,
}

pub struct RenderWarning {
    pub kind: WarningKind,
    pub node_id: Option<NodeId>,
    pub details: String,
}

pub enum WarningKind {
    /// Consumer の resolver が fallback を返した (Ok(fallback_intrinsic))
    ResolverFallback { fragment_id: Symbol },
    /// Consumer の network が fallback を返した
    NetworkFallback { url: Url },
    /// Policy violation を Consumer の on_violation が Warn 扱いにした
    PolicyWarning { violation: PolicyViolation },
    /// target-* 参照先が見つからず fallback_text で描画された
    UnresolvedTarget { fragment_id: Symbol },
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

// ── Public API forward-compatibility convention (round 3 review 対応) ─
// 
// [rule]
//   将来 variant / field 追加の余地がある全 pub enum / struct に
//   #[non_exhaustive] を付与し、Consumer 側 exhaustive match / literal
//   construction の accidental breakage を防ぐ。
// 
// [applies to] (raikiri-traits の全 pub enum は原則対象、round 3 review #3
//   訂正で hand-maintained list は廃止)
//   ルール: 「spec / semantics で "完全な集合" であると証明できない enum は
//           全て #[non_exhaustive]」
//
// [exceptions] (justification 必須、doc の enum 定義箇所で理由を明示)
//   現時点で確認できる exception は無し。
//   - Method: HTTP method は仕様上拡張可能 (round 3 review #3 訂正、
//     以前 exception に誤分類していた) → #[non_exhaustive] 対象
//   - WarningKind の list 重複、TargetConvergence の list 残存は削除 (前 doc
//     の bug、round 3 review #3 で指摘)
//
// [struct construction pattern] — round 3 review #2 対応
//   #[non_exhaustive] な struct は外部 crate から literal construction が
//   できないため、必ず以下を提供する:
//     1. `impl Default for Config` (全 field の妥当な default)
//     2. `impl Config { pub fn new() -> Self { Self::default() } }`
//     3. mutation pattern を doc に明示: `let mut c = Config::default();
//        c.widow_line_buffer = 5;`
//     4. field 数が多い場合は追加で builder pattern を提供
//        `Config::builder().widow_line_buffer(5).build()`
//   外部 crate から `Config { .. Default::default() }` は不可能なので
//   推奨しない (compile error)。
//
// [enforcement]
//   Section 12 で "public-api-compile-tests" として external consumer crate
//   を模擬した test を CI で実行、全 pub struct が external から constructable
//   かを検証。

// [対象 struct] (round 3 Missing #6 対応、全 non-exhaustive pub struct に適用)
// すべての pub config struct に以下 3 点を提供:
//   1. impl Default        - 全 field の妥当な default
//   2. impl Config { fn new() }  - Default 相当の shortcut
//   3. Optional: ConfigBuilder  - field 数が多い / 相互依存がある場合
//
// 適用対象:
//   LookaheadConfig, StreamingConfig, BatchConfig, ParseOptions,
//   PageDefaults, PaintOptions, LookaheadConfig 経由の間接 config

pub struct LookaheadConfigBuilder { /* fluent field-setter chain、build() で LookaheadConfig を返す */ }
impl LookaheadConfig {
    pub fn builder() -> LookaheadConfigBuilder { /* ... */ }
    pub fn new() -> Self { Self::default() }
}
impl Default for LookaheadConfig { /* 全 field の default 値 */ }

// Same pattern for other configs (省略):
// StreamingConfigBuilder, BatchConfigBuilder, ParseOptionsBuilder,
// PageDefaultsBuilder, PaintOptionsBuilder

// ── 中立モデル型 (crate 境界を跨いで参照される) ────────────────
pub struct PageFragment { /* ... §11.2 参照 */ }
pub struct PageBox      { /* ... §9    参照 */ }
pub struct PageContext  { /* ... §7.2  参照 */ }
pub struct LayoutBuffer { /* ... §5    参照 */ }
pub struct TargetRegistry { /* ... §7.2 参照 */ }
#[non_exhaustive]
pub enum   GcpmDirective { /* ... §7.1  参照 */ }
pub struct LookaheadConfig {
    pub widow_line_buffer: usize,
    pub orphan_line_buffer: usize,
    pub break_avoid_max_subtree_blocks: usize,
    pub max_container_probe_pages: Option<usize>,
    pub allow_cross_size_lookahead: bool,
}

// ── Entry point config (Finding #5 対応: raikiri 内 iteration 廃止) ────

/// 全 entry point (plan / render_streaming / render_batch) が受け取る
/// resource / cost 上限。 round 4 review #1 対応で `BatchConfig` から
/// `RenderLimits` に昇格、`plan` と Streaming にも同じ bound を強制。
#[non_exhaustive]
pub struct RenderLimits {
    pub max_document_pages: Option<u32>,          // 超過 → LimitExceeded { kind: Pages }
    pub max_dom_nodes: Option<u64>,               // parse 完了後 check
    pub max_target_slots: Option<u32>,            // per-doc target 参照数上限
    pub max_layout_buffer_entries: Option<u32>,   // LayoutBuffer に貯める上限
    pub max_aggregate_bytes: Option<u64>,         // approximate memory footprint
}
impl Default for RenderLimits { /* 妥当な defaults (fulgur 想定): pages=10000, nodes=1M, slots=100k, buffer=10k, bytes=1GB */ }
impl RenderLimits {
    pub fn new() -> Self { Self::default() }
    pub fn builder() -> RenderLimitsBuilder { RenderLimitsBuilder::default() }
}
pub struct RenderLimitsBuilder { /* fluent chain */ }

#[non_exhaustive]
pub struct PlanConfig {
    pub lookahead: LookaheadConfig,
    pub limits: RenderLimits,             // round 4 review #1 対応
    pub initial_registry: Option<TargetRegistry>,  // round 4 review #2 対応: 反復 chain 用
}

#[non_exhaustive]
pub struct StreamingConfig {
    pub lookahead: LookaheadConfig,
    pub limits: RenderLimits,             // round 4 review #1 対応
    pub initial_registry: Option<TargetRegistry>,
}

#[non_exhaustive]
pub struct BatchConfig {
    pub limits: RenderLimits,             // max_document_pages を吸収、round 4 review #1 対応
    pub initial_registry: Option<TargetRegistry>,
}

// ── Plan mode (dry-run、box tree 構築なし) ────────────────────

pub struct DocumentPlan {
    pub total_pages: u32,
    /// hint: render 時は再計算される
    pub target_registry: TargetRegistry,
    pub target_definitions: Vec<TargetDefinition>,
    pub unresolved_targets: Vec<UnresolvedTarget>,
    pub page_summary: Vec<PageSummary>,
    // ★ PaintedBox tree は含まない (本当に plan だけ)
}

pub struct PageSummary {
    pub page_index: u32,
    pub page_box: PageBox,
    pub break_reason: BreakReason,
    pub target_slot_count: u32,
    pub target_definition_count: u32,
    pub content_height: f32,
    // ★ box tree、glyph 情報なし
}

// ── Discrepancy 型 (Consumer 収束判定用) ────────────────────

pub struct TargetDiscrepancy {
    pub fragment_id: Symbol,
    pub hinted_page: Option<u32>,
    pub actual_page: u32,
    pub hinted_text: Option<String>,
    pub actual_text: String,
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

// Advanced driver: strategy を直接指定 (new review #5 対応、&Document 前提)
pub fn render_with<L, T, E, R>(
    doc: &Document,
    defaults: PageDefaults,
    resolver: &dyn ReplacedResolver,
    sink: &mut dyn RenderSink,
    lookahead: L,
    target: T,
    emission: E,
    reflow: R,
) -> Result<RenderStatus, RenderError>
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

`NetworkProvider` / `ReplacedResolver` 実装群 + policy wrapper (Finding #6 対応)。

**Base provider 実装**:
- `NoOpNetworkProvider`: fulgur デフォルト、全 fetch を reject

**Policy wrappers** (decorator pattern):
- `SandboxedNetProvider<P: NetworkProvider>`: 任意の provider を wrap し、
  ResourcePolicy を pre-fetch / post-fetch で検証
- `SandboxedResolver<R: ReplacedResolver>`: 同じく resolver を wrap

**Policy preset 実装**:
- `DenyAllPolicy`: 全 fetch を reject する最も restrictive な policy
- `DefaultSandboxPolicy`: 実用的な出発点
  - `allowed_schemes: ["https", "data"]`
  - `max_fetch_bytes: 10 MB`
  - `max_decoded_bytes: 100 MB`
  - `fetch_timeout: 5s`
  - `max_import_depth: 4`

```rust
// raikiri-net の主要 API
pub struct SandboxedNetProvider<P: NetworkProvider> { /* ... */ }
impl<P: NetworkProvider> SandboxedNetProvider<P> {
    pub fn new(inner: P, policy: Arc<dyn ResourcePolicy>) -> Self;
    pub fn with_default_sandbox(inner: P) -> Self;
    pub fn inner(&self) -> &P;
}
impl<P: NetworkProvider> NetworkProvider for SandboxedNetProvider<P> {
    fn fetch(&self, request: Request) -> Result<FetchedResource, NetworkError> {
        // Pre-fetch: scheme, host, redirect の検証
        // Fetch: AbortSignal を policy.fetch_timeout で強化
        // Post-fetch: size, MIME の検証
        // Consumer の inner に委譲
    }
}

pub struct SandboxedResolver<R: ReplacedResolver> { /* ... */ }
// 同じ shape

pub struct DenyAllPolicy;
pub struct DefaultSandboxPolicy { /* ... */ }
```

**wrapper composition** で複数の cross-cutting concerns を合成可能:
```rust
let np = LoggingProvider::new(
    SandboxedNetProvider::new(
        FulgurNetProvider::new(assets_dir),
        Arc::new(DefaultSandboxPolicy::default()),
    ),
);
```

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
    LookaheadConfig, StreamingConfig, BatchConfig,
    DocumentPlan, PageSummary, TargetDiscrepancy,
    ContainerOverflowFallback, ReflowAction, DirtyDeadline,
};
pub use raikiri_paint;  // sub-module として

/// parse → cascade → Document 組立て (orchestrator)
/// round 4 review #4 対応: 返り値を RenderError に統一 (Parse / Cascade / Configuration /
/// Io 等をそのまま伝播できる)
pub fn parse_html<R: std::io::Read>(input: R, options: &ParseOptions) 
    -> Result<Document, RenderError> {
    let uncascaded = raikiri_html::parse(input, options)
        .map_err(RenderError::Parse)?;
    let cascade = raikiri_style::cascade(&uncascaded, options.extra_stylesheets)
        .map_err(RenderError::Cascade)?;
    Ok(raikiri_dom::Document::assemble(uncascaded, cascade))
}

// ── 3 entry point (Finding #5 対応、new review #5 訂正済み) ─────

/// dry-run: parse + cascade + layout planning のみ (paint scene / PaintedBox 構築なし)
/// 用途: fulgur の pass-1 前哨、cost 見積り、target 収束判定用の hint 生成
/// **入力は事前 parse 済みの &Document** (input の再利用問題を回避、new review #5 対応)
/// round 4 review #1, #2 対応: PlanConfig で limits と initial_registry を受け取る
pub fn plan(
    doc: &Document,
    defaults: PageDefaults,
    resolver: &dyn ReplacedResolver,
    config: PlanConfig,
) -> Result<DocumentPlan, RenderError>;

/// Streaming 向け: BoundedLookahead + PlaceholderTargetResolver + ImmediateEmission
/// StreamingConfig.initial_registry で plan の結果を hint として渡す
pub fn render_streaming(
    doc: &Document,
    defaults: PageDefaults,
    resolver: &dyn ReplacedResolver,
    config: StreamingConfig,
    sink: &mut dyn RenderSink,
) -> Result<RenderStatus, RenderError>;

/// Batch 向け: UnboundedLookahead で Fragmentation L3 準拠の 1 pass
/// BatchConfig.initial_registry で plan の結果を hint として渡す
pub fn render_batch(
    doc: &Document,
    defaults: PageDefaults,
    resolver: &dyn ReplacedResolver,
    config: BatchConfig,
    sink: &mut dyn RenderSink,
) -> Result<RenderStatus, RenderError>;

// ── Convenience wrapper (impl Read から一発で render) ──────────
// 単発 rendering の便利関数。plan + render の 2 pass を行いたい場合は
// Consumer が明示的に parse_html + plan + render_* を chain する
pub fn render_streaming_from_html(
    input: impl std::io::Read,
    options: &ParseOptions,
    defaults: PageDefaults,
    resolver: &dyn ReplacedResolver,
    config: StreamingConfig,
    sink: &mut dyn RenderSink,
) -> Result<RenderStatus, RenderError> {
    let doc = parse_html(input, options)?;
    render_streaming(&doc, defaults, resolver, config, sink)
}

// dogfooding helper (VRT や examples 用途)
pub fn html_to_png(html: &str) -> Result<Vec<u8>, RenderError>;
```

**3 entry point の重要な性質** (Finding #5 対応、DoS 耐性):
- 各 entry point は raikiri 内部で **1 pass 固定**
- `initial_registry` は **hint** のみ、render は必ず自分で target-* を再計算
- 収束が必要な用途は Consumer が `plan` + `render_*` を chain して自分の
  iteration bound で管理 (untrusted 入力なら iter=0、trusted なら iter=N)
- `TargetConvergence` enum、`NPassConverge` は raikiri 側では未実装 (Non-goal)

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

### 5.2 Consumer 介入ポイント (TreeSink wrap = Layer 1 DOM sanitize)

**Security 2 層 model の Layer 1** (Finding #6 対応、詳細は §10.1)。
Layer 2 (resource-level policy) は §10 の SandboxedNetProvider / SandboxedResolver
で扱う。両方の Layer が揃わないと server-side safety は成立しない。

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
    ParseOptions, PageDefaults, LookaheadConfig, StreamingConfig, BatchConfig,
    parse_html, plan, render_streaming, render_batch, RenderStatus, RenderError,
};

let options = ParseOptions {
    extra_stylesheets: &[fulgur_style],
    network: Some(&NoOpNetworkProvider),
    base_url: Some(base),
};

// ── parse を 1 度だけ実行し、&Document を plan と render で共有 ─
// (new review #5 対応、input の再利用問題を回避)
let doc = raikiri::parse_html(html_input, &options)?;
let resolver = FulgurResolver::new(font_data, images);

// ── plan (dry-run、DoS 防御としても有用) ───────────────
let plan = raikiri::plan(&doc, PageDefaults::a4(), &resolver, LookaheadConfig::default())?;

// DoS 防御: 事前に document size をチェックして render を拒否可能
if plan.total_pages > fulgur_config.max_pages_per_request {
    return Err(FulgurError::DocumentTooLarge { pages: plan.total_pages });
}

// ── Streaming (fulgur デフォルト、大規模ドキュメント向け) ─
// round 4 review #5 対応: #[non_exhaustive] struct は Default 経由の mutation で構築
let mut streaming_cfg = StreamingConfig::default();
streaming_cfg.initial_registry = Some(plan.target_registry.clone());
raikiri::render_streaming(
    &doc, PageDefaults::a4(), &resolver,
    streaming_cfg,
    &mut fulgur_pdf_sink,
)?;

// ── Batch (Fragmentation L3 準拠、multi-page flex/grid 対応) ─
let mut batch_cfg = BatchConfig::default();
batch_cfg.limits = RenderLimits::builder().max_document_pages(100).build();
batch_cfg.initial_registry = Some(plan.target_registry);
raikiri::render_batch(
    &doc, PageDefaults::a4(), &resolver,
    batch_cfg,
    &mut fulgur_pdf_sink,
)?;
```

**3 entry point の使い分けガイド**:

| Consumer が欲しいもの | 使う API |
|---|---|
| target 収束のための hint (最低限、DoS 防御) | `plan` |
| fulgur pass-1 (Drawables アクセス、layout 情報) | `render_streaming` + inspection sink |
| 実際の PDF/画像生成 | `render_streaming` / `render_batch` + Consumer sink |
| Fragmentation L3 準拠 (multi-page flex/grid) | `render_batch` (unbounded lookahead) |
| VRT / debug raster | `render_streaming` + `raikiri-paint::paint_to_scene` |

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

### 5.4.1 Snapshot semantics for parallel margin-box layout (Missing consideration 対応)

Parallel margin box layout は GCPM state (counter tree、named string、target
registry、running bindings) を読むが、書き込みはしない。以下の contract で
concurrency 安全性を保証:

**PageContext snapshot 手順**:
1. per-page loop の "Layout フェーズ" (§5.1) 完了直後 = margin box layout の
   直前に、`PageContext` の **immutable snapshot** を取る
2. Snapshot は `Arc<GcpmSnapshot>` として全 margin box 並列 worker に共有
3. Snapshot の中身: counter tree の deep copy、strings の 4-snapshot、running
   bindings、target registry の read-only view
4. 並列 worker は snapshot のみ read、`PageContext` 本体は書き込まない

```rust
/// **All fields are OWNED immutable clones. No interior mutability, no
/// references to shared mutable storage. Constructing this type is the
/// well-defined "epoch" boundary; once created, mutation of the source
/// PageContext / TargetRegistry cannot affect this snapshot.**
/// (round 3 review #4 対応)
struct GcpmSnapshot {
    counters: HashMap<Symbol, CounterStack>,          // owned deep copy
    strings: HashMap<Symbol, NamedStringSnapshot>,    // owned deep copy
    running: HashMap<Symbol, RunningTemplateId>,      // owned deep copy
    // ★ 訂正: 以前 "TargetRegistryView (read-only wrapper)" と書いたが、
    // 内部が shared mutable への reference だと snapshot にならない。
    // owned immutable snapshot に修正:
    targets: OwnedTargetRegistrySnapshot,             // owned deep copy
    page_index: u32,
    page_name: Option<Symbol>,
}

/// TargetRegistry の time-point snapshot。owned な HashMap 群のみを持ち、
/// 元の TargetRegistry がその後 mutate しても影響を受けない。
struct OwnedTargetRegistrySnapshot {
    resolved: HashMap<Symbol, TargetInfo>,   // owned clone
    // pending_slots は snapshot 時点で fix、以降 mutate されない
    pending_slots_at_snapshot: Vec<TargetSlot>,
}

// per page loop, layout フェーズ後:
let snapshot = Arc::new(GcpmSnapshot::from(&page_context));

let margin_fragments: [Option<MarginBoxFragment>; 16] = (0..16)
    .into_par_iter()
    .map(|slot_idx| {
        let snap = Arc::clone(&snapshot);
        // 各 worker は snap を read-only で使用、PageContext 本体は触れない
        layout_margin_box(slot_idx, &snap, &running_store, &font_context)
    })
    .collect_into_slice(...);

// 並列完了後、page_context は変更されないまま
```

**共有される他のリソース**:
- **`ParsedRunningTemplate`**: `Arc` で共有、read-only
- **`FontContext` (parley)**: M0 で `Sync` 確認済み前提。`Arc` で共有
- **`ComputedValues`**: `Arc<ComputedValues>` で共有、read-only

**Per-worker owned mutable state** (round 3 Missing #5 対応):

Margin box 内でも CSS spec 上 `counter-increment` / `counter-set` の記述は
許容される (spec: 各 margin box が独立の counter scratch を持ち、他の margin
box や body content に影響しない)。そのため:

```rust
struct MarginBoxCounterScratch {
    /// 各 margin box worker が独自に持つ counter mutation buffer。
    /// GcpmSnapshot の counters を初期値として clone し、margin box 内での
    /// increment/set 操作を吸収。layout 完了時に discard される。
    local_counters: HashMap<Symbol, CounterStack>,
    /// string-set は margin box 内では使えない (spec) ため管理しない
}

impl MarginBoxCounterScratch {
    fn new_from_snapshot(snap: &GcpmSnapshot) -> Self {
        Self { local_counters: snap.counters.clone() }
    }
}
```

- 各 parallel worker は自分の `MarginBoxCounterScratch` を new_from_snapshot で
  作成、margin box 内の counter 操作は scratch に対して行う
- 他 margin box worker の scratch とは independent (書き込み共有なし)
- Margin box layout 完了時に scratch は drop、他 margin box や次ページに影響
  しない (spec 準拠)

**書き込み禁止の enforcement (round 3 review #4 訂正)**:
- `GcpmSnapshot` の全 field は owned + immutable primitive/collection のみ
  (`Cell` / `RefCell` / `Mutex` / `Arc<Mutex<...>>` を含まない)
- `#[derive(Clone)]` して `Arc::new(snapshot)` で共有、`Arc<GcpmSnapshot>` 内は
  当然 immutable
- Rayon closure は `Arc<GcpmSnapshot>` を clone して受け取る
- `PageContext` は per page loop の main thread のみが `&mut` を持ち、snapshot
  作成後は margin box layout 完了まで touch しない (API contract で保証)
- **具体的な lint 名 (round 3 review #4 指摘)**: 
  - `clippy::mut_from_ref` (interior mutability の誤用検知)
  - Custom lint: `raikiri-lints::no_interior_mut_in_snapshot` (`Cell` / `RefCell`
    等の pattern を snapshot 型内で禁止、raikiri-wpt の CI で enforce)
- **compile-time enforcement は "型設計 + API contract"** であって、Rust の型
  system 単体では interior mutability を完全禁止できないため、追加で lint と
  concurrency test で validation する (§12 参照)

**Consumer resolver / network の並列呼び出しについて** (round 3 review #5 対応):
- `ReplacedResolver` は sequential body layout phase 内でのみ呼ばれる仕様。
  並列 margin box worker からは呼び出さない
- しかし running template 内の `<img>` / `<svg>` / `<math>` などの replaced
  content は margin box layout 時にサイズが必要 → **pre-resolution stage** を
  導入して解決:

**Running template replaced content の pre-resolution 仕様**:

1. Cascade 完了時 (`§5.1 Phase A` 終了時)、raikiri-style は各
   `RunningTemplate` の subtree を scan して replaced element を列挙し、
   `pending_running_resources: Vec<ReplacedElementRef>` として Document に格納
2. Phase B の per-page loop 開始前 (最初の page の layout 開始前) に、
   raikiri-dom が sequential に全 pending_running_resources を resolver で
   pre-resolve、結果を `resolved_running_resources: HashMap<NodeId, IntrinsicBox>`
   としてキャッシュ
3. margin box layout 時、running template 内の replaced element は cached
   `IntrinsicBox` を lookup するのみ (resolver は呼ばれない → 並列安全)
4. Consumer 責任: 大量の running template 内 image の resolve は Consumer の
   Layer 2 policy で bound (§10.1 の SandboxedResolver でサイズ・件数制限)

**Fallback / error 挙動**:
- Consumer resolver が Err を返した replaced element: `IntrinsicBox::fallback()`
  で解決、`RenderSummary.warnings` に `WarningKind::ResolverFallback` として記録
  (§5.5 の recoverable pattern と同じ)
- pre-resolution 段階で resolver が全て失敗しても layout は継続 (Consumer 判断)

**代替設計 (Non-goal で扱わない)**:
- Running template 内の replaced content を明示的に非対応にする案は却下。
  実務で "header にロゴ画像" は極めて典型的、これを無視すると使えない

`NetworkProvider: Send + Sync` は blitz-traits 準拠 (trait bound 済み)、
pre-resolution stage で並列 fetch も許容 (ただし Consumer 実装内での並列は
Consumer が管理)。

### 5.4.2 Overflow-safe budget と deadline (round 3 Missing #2 対応)

Untrusted document を Consumer が処理する際の安全パターンを doc レベルで
規定 (raikiri 側 API は最小限):

**Overflow-safe budget calculations (Consumer 責務)**:
- Page 数 × per-page cost の累算は `u64` 前提、`saturating_add` を使う
- `checked_mul` で overflow を明示的に検知
- Consumer が予算 tracker を実装するときの推奨: `Saturating<u64>` newtype

**Render deadlines (`AbortSignal` 経由、raikiri API)**:
- Consumer が `AbortController::with_timeout(Duration)` で timer を作成
- `AbortSignal` を `Request` / `ResolverRequest` / render 各所で参照
- raikiri は各 loop iteration の先頭で `signal.is_aborted()` を check
- `is_aborted() = true` なら:
  - Streaming: 直前まで committed pages を残して `RenderStatus::Aborted` で
    return
  - Batch: pre-emission phase なら sink 未使用、post-emission phase なら
    そこまで emit + `Aborted` return

**Cancellation の semantics (round 3 Missing #2 対応)**:
- `AbortSignal` は cooperative (Consumer が signal.abort() を呼ぶ or timer
  期限で auto abort)
- raikiri は AtomicBool を acquire で読む (blitz と同じ semantics)
- 中断ポイント (中断可能な場所):
  - per-page loop の先頭
  - LayoutBuffer の decision point
  - Rayon parallel margin box の spawn 前
  - Resolver 呼び出し前後
- 中断できない場所 (原子的にやり切る):
  - 単一 `accept_page` 内の描画 (Consumer 責任)
  - Sink の finish_render 呼び出し (呼ばれた時点で完了する)

**Untrusted 文書での error 挙動 (Layer 1/2 の話とも重複)**:
- untrusted HTML: Layer 1 sanitize で削除、raikiri に到達する時点で構造安全
- untrusted CSS: Layer 1 sanitize + `ResourcePolicy` で fetch 制限
- untrusted resource: `SandboxedResolver` / `SandboxedNetProvider` で fetch 制限
- Consumer が `AbortController::with_timeout` で walltime bound
- `max_document_pages` で page 数 bound
- 上記を組み合わせれば worst-case は bounded、raikiri 単独 guard は追加しない

これで **PageContext の parallel margin box layout 中の状態変化は起こらず、
byte-identical goal と concurrency safety が両立** する。

### 5.5 Error / Partial output semantics (Finding #10 対応、new review 対応済)

`render_*` は `Result<RenderStatus, RenderError>` を返す。すべての `RenderError`
variant は **terminal** (rendering がそこで停止)。`Aborted` は error でなく
`RenderStatus::Aborted` として graceful stop を表現。

#### `RenderStatus` (成功系)

```rust
pub enum RenderStatus {
    /// 全ページ emit 完了、finish_render も成功
    Completed(RenderSummary),
    /// AbortSignal 起源の graceful shutdown。partial pages emit 済み、
    /// finish_render は呼ばれていない。RenderSummary は取れないので
    /// partial_pages のみ通知
    Aborted { partial_pages: u32 },
}
```

#### `RenderError` (すべて terminal)

**Consumer 側 fallback は resolver / network の Ok(fallback_value) 返却で表現**。
raikiri は fallback 発生を `RenderSummary.warnings` に記録。Consumer が Err を
返せば raikiri は該当 variant で terminal。

#### Emission 開始前 vs 開始後の error 発生位置 (新review #2 対応)

| Error 発生位置 | Streaming preset | Batch preset |
|---|---|---|
| Parse / Cascade / Configuration (pre-emission) | sink 未使用 | sink 未使用 |
| Layout / Policy / Resolver / Network (pre-emission phase 内) | sink 未使用 | sink 未使用 |
| PageLimitExceeded (per-page loop 中) | 直前まで `accept_page` 済み | Batch buffer 蓄積中、`accept_page` 未呼び出し |
| `accept_page` 内で Sink エラー | 該当 page で fail、以前は committed | Batch 内で emit 途中、以前 page は committed |
| `finish_render` 内で Sink エラー | 全 page committed、finish 未完了 | 全 page committed、finish 未完了 |

**重要な訂正**: 
- Streaming の `accept_page` 中に error が発生した場合、その page は committed
  されない (raikiri が accept_page を呼んで Err が返った時点で halt)。以前 emit
  済みの page は committed 状態
- Batch の "all-or-nothing" は **emission 開始前の error に限定**。Batch buffer
  から emission が始まった後 (accept_page が呼ばれ始めた後) の error は partial
  状態を残す
- 真の atomic guarantee が必要な場合、Consumer が RAM buffer に貯めて自前で
  commit する (raikiri は transactional sink API を提供しない、Consumer の
  自由度を残す)

#### Plan mode (`plan`)

- Error 発生時、`DocumentPlan` は返らない
- **sink emit なし** (dry-run、sink を持たない)
- ただし resolver / network provider は呼ぶ (Consumer 側の fetch state に副作用
  あり得る)

#### Consumer 側 fallback pattern

Consumer の resolver / network 実装が `fallback` を返せる場合、raikiri は success
として扱い、`RenderSummary.warnings` に記録:

```rust
impl ReplacedResolver for FulgurResolver {
    fn resolve(&self, req: ResolverRequest<'_>) -> Result<IntrinsicBox, ResolverError> {
        match self.actual_resolve(req) {
            Ok(intrinsic) => Ok(intrinsic),
            Err(_) if self.allow_fallback => {
                // Consumer が fallback を選択、raikiri は Ok として扱う
                // raikiri 側は自動的に RenderWarning::ResolverFallback を summary に追加
                Ok(IntrinsicBox::fallback_missing_image())
            }
            Err(e) => Err(e),  // Consumer が fallback を拒否 → raikiri terminal
        }
    }
}
```

Consumer が `Ok(fallback)` を返した場合、raikiri は `RenderWarning` を生成し
`RenderSummary.warnings` に集める。Consumer は Err を返しても Ok を返してもよい
(policy 選択)。

#### fulgur example (partial handling、new review 対応)

```rust
struct FulgurPdfSink {
    krilla_doc: krilla::Document,
    pages_committed: Vec<u32>,
    finalized: bool,
}

impl RenderSink for FulgurPdfSink {
    fn accept_page(&mut self, page: PageFragment) -> std::io::Result<()> {
        self.pages_committed.push(page.page_index);
        // draw ...
        Ok(())
    }
    fn finish_render(&mut self, summary: RenderSummary) -> std::io::Result<()> {
        self.finalized = true;
        // patch pending target slots ...
        Ok(())
    }
}

impl FulgurPdfSink {
    fn finalize_pdf(self) -> Result<Vec<u8>, FulgurError> {
        if !self.finalized {
            return Err(FulgurError::PartialRender {
                committed_pages: self.pages_committed,
            });
        }
        Ok(self.krilla_doc.into_bytes()?)
    }
}

// Consumer 主導フロー (round 4 review #5 対応: RenderStatus は #[non_exhaustive]
// なので wildcard arm 追加)
let mut sink = FulgurPdfSink::new(krilla_doc);
match raikiri::render_streaming(&doc, defaults, resolver.as_ref(), config, &mut sink) {
    Ok(RenderStatus::Completed(summary)) => {
        for warning in &summary.warnings {
            log::warn!("{:?}", warning);
        }
        let pdf_bytes = sink.finalize_pdf()?;
        return Ok(pdf_bytes);
    }
    Ok(RenderStatus::Aborted { partial_pages }) => {
        log::info!("Aborted after {} pages", partial_pages);
        drop(sink);
        return Err(FulgurError::Aborted);
    }
    Ok(_) => {
        // 将来 variant 追加時の forward-compat (round 4 #5)
        drop(sink);
        return Err(FulgurError::UnknownRenderStatus);
    }
    Err(RenderError::LimitExceeded { kind, limit, actual }) => {
        drop(sink);
        return Err(FulgurError::LimitExceeded { kind, limit, actual });
    }
    Err(e) => {
        drop(sink);
        return Err(FulgurError::RenderFailed(e));
    }
}
```

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

### 7.3 Running element templates (Finding #8 対応)

`position: running(name)` された subtree は body flow から除去、
`RunningTemplateStore` として保持。@page margin box の `content: element(name)` で
参照。

**2 tier キャッシュ設計** (Finding #8 対応):

```rust
// raikiri-dom 内、crate-private
struct RunningTemplateStore {
    parsed_templates: HashMap<RunningTemplateId, ParsedRunningTemplate>,
    // layout 結果はキャッシュしない
}

struct ParsedRunningTemplate {
    subtree_root: NodeId,             // Dom 内のポインタ
    computed_styles: Arc<CascadeSubset>,  // 事前 cascade 済み
    directives: Vec<GcpmDirective>,       // GCPM 動的部分
    dynamic_flags: DynamicFlags,          // 何が dynamic か
}

struct DynamicFlags {
    has_counter: bool,      // counter() / counters() 参照あり
    has_string: bool,       // string() 参照あり
    has_target: bool,       // target-* 参照あり
    has_content_variant: bool,  // content(before/after) 参照あり
}
```

**方針**:
- **cascade 済み parsed template のみ cache** (Dom 内 subtree ポインタ + 事前
  cascade 結果 + directive)
- **layout 結果は cache しない**、常に per-page で layout
- **理由 (前版からの訂正)**: mixed-size ページで margin box の width/height が
  変わると、静的 content でも実効寸法が違い、座標変換のみでは正しくない。
  Cache key に (page_name, effective_margin_box_geometry, cascade_context) 等を
  含めるのは複雑度が跳ねる。per-page re-layout は 1 page 分の text shape 程度で
  低コスト (§5.4 の rayon 並列で吸収可能)、correctness 優先

**layout 時の flow**:
1. margin box の実効 geometry `(page_name, effective_width, effective_height, page_context)`
   を PageStream が確定 (§9.1 の page name 遷移で決まる)
2. `parsed_templates` から `ParsedRunningTemplate` を取得
3. `layout_running_template(parsed, page_ctx, margin_box_geom)` で常に per-page layout
4. 結果を `MarginBoxFragment` として emit

**`DynamicFlags` の使い所**: 完全に static な template (`dynamic_flags = all false`)
は同一 margin box geometry 内で複数ページ跨いで cache 化可能。これは post-M8 の
最適化として保留 (M1〜M8 は常に re-layout)。

### 7.3.1 Running template の DoS 耐性は Consumer 責務 (round 3 review 対応)

Per-page re-layout により、running template の実装コストは asymptotic に
**`O(pages × template subtree size)`** で amplify されるが、この worst case を
raikiri 側の guard で防ぐのは **責務の重複** となるため行わない。理由:

1. **Layer 1 sanitize が既に防いでいる**: Consumer が TreeSink wrap で
   untrusted HTML の DOM 巨大化を防ぐ (§5.2)。running template の subtree size
   もこの層で bound される
2. **`max_document_pages` が既に aggregate work を bound**: `BatchConfig` /
   Consumer iteration で page 数 upper bound を設定できる。`pages ×
   template_size` の積は、この page 上限と Consumer sanitize の template
   size 上限の 2 つの制約で自然に bound
3. **加えて Consumer 側で timeout / cancellation**: `AbortSignal` を使って
   Consumer が deadline を強制 (§10)

**raikiri の役割**:
- **fail-fast は自ら enforce しない** (Consumer 責務)
- observability として、running template の per-page layout cost を
  `RenderSummary` の debug field に記録する余地は将来残す (post-M8)
- Consumer が自分の Layer 1/2 sanitize + max_document_pages を組み合わせて
  asymptotic bound を設計するのが正しい形

**Guard mechanism (削除)**: 前版で `max_running_template_nodes` /
`max_running_template_bytes` を LookaheadConfig に足す提案をしたが、Consumer
責務との重複により削除。同種の "content shape に対する raikiri 側 guard" は
今後も原則入れない (fail-fast 対象は raikiri 内部の異常のみ)。

**Per-page CPU 目安 (参考)**:
- 通常 header/footer template: 数十 node、per-page layout ~1ms 程度
- 1000 ページ document: 累計 ~1s 追加 (許容範囲)
- untrusted 大 template を防ぎたい Consumer は、TreeSink wrap で subtree size
  を validate (`position: running(...)` された subtree の node 数を数える等)

**Post-M8 optimization** (§7.3 参照): `dynamic_flags = all false` な template
の layout 結果 cache 導入で、上記 worst case を大幅緩和できる予定。

### 7.3.2 Node counting / byte sizing の stable definitions (round 3 Missing #1 対応)

raikiri 側で running-template guards を持たない (§7.3.1) ため、"stable node
count / byte size" の定義は現状 raikiri の API 面に露出しない。ただし将来
observability として `RenderSummary` に debug field を追加する余地を残しつつ、
定義を先取りしておく:

**Node count 定義 (将来 exposed の場合の準備)**:
- Element node、Text node、Comment node、Attribute はカウント対象
- **Shared subtree の扱い**: `position: running(name)` された subtree は
  running template pool と body flow の両方から見えるが、**pool 側で 1 回だけ
  count**、body flow 側では 0
- **`content` プロパティで生成される content**: cascade 時に決まる ContentValueItem
  列は 1 node として count (実際の text 長は無視)
- Resource payload (image bytes、SVG source 等) は node count 対象外

**Byte size 定義 (将来 exposed の場合の準備)**:
- Serialized approximate bytes = `mem::size_of::<Node>() * node_count +
  Σ text_content.len() + Σ attr_value.len()`
- Rust の型サイズを "approximate" とし、正確な heap allocation は数えない
- **Shared subtree の扱い**: 上と同じ、pool 側で 1 回のみ
- Resource payload は「payload byte 数 (fetch 前は 0)」として別 field で
  記録する余地

**Consumer 側での測定 (推奨)**:
- Consumer が Layer 1 sanitize で DOM walk するタイミングで自前の counter を
  実装可能 (raikiri は生の Dom へアクセス可能な API を提供、TreeSink wrap 内で
  count 可能)
- raikiri が debug field を提供するのは post-M8 の話。M1〜M8 では Consumer が
  自前で数える

これらの定義は現状 spec に露出しないが、将来 `RenderSummary.debug_stats` 等の
opt-in field を足す際の contract として先取りしておく。

### 7.4 target-* 解決の 2 戦略 + Consumer iteration (Finding #5 対応)

TargetResolver trait の実装として 2 種類のみ提供。**raikiri 内部での iteration
収束は行わない** (DoS 耐性のため)。収束が必要な用途は Consumer が
`plan` + `render_*` を chain して自分の iteration bound で管理する。

**`PlaceholderTargetResolver` (Streaming preset default)**

- 未解決の target-* に到達したら `TargetSlot { id: TargetSlotId, ... }` を発行
- **`TargetSlotId = (page_index, sequence)`** で stable な識別（Finding #4 対応）
- `ResolvedContent::TargetSlot(slot_id)` として PageFragment.target_slots に emit
- **PageFragment.target_definitions に "このページで定義された fragment id" を並行 emit**
- `RenderSink::finish_render(summary)` で Consumer が最終 `TargetRegistry` を受取り
- Consumer (fulgur → krilla) が Form XObject slot として PDF patch
- streaming 保持

**`RegistryTargetResolver` (`initial_registry` に hint を渡した時のみ有効)**

- `StreamingConfig.initial_registry` / `BatchConfig.initial_registry` の `Some(...)`
  を受け取り、target-* を **hint 値で** 解決する
- **重要**: render は hint に依らず layout 時に自分で target-* を実測し直す。
  hint と actual に差があれば `RenderSummary.target_discrepancies` に記録
- Consumer は discrepancies を見て収束判定 (次 iteration が必要かを決める)

**削除された概念** (Finding #5 対応):
- ~~`ConvergingTargetResolver` / `NPassConverge`~~ - 攻撃者による oscillation 誘発
  リスクのため raikiri 側では未実装。Consumer が `plan` + `render_*` を
  chain して自分で iteration
- ~~`TargetConvergence` enum~~ - Consumer が iteration bound を管理

### 7.4.1 Consumer 収束アルゴリズム (round 4 review #2 対応、明確化)

**アルゴリズム** (untrusted input 用):

```
1. doc = parse_html(input, options)                       — 1 回のみ
2. prev = None
3. loop iter in 0..fulgur_config.max_target_iterations:
     next_plan = plan(&doc, defaults, resolver, PlanConfig {
         lookahead, limits,
         initial_registry: prev,   ← 前 iter の結果を hint に (round 4 #2 対応)
     })
     if next_plan.target_registry == prev.unwrap_or(default):
         break                                            — 収束
     prev = Some(next_plan.target_registry)
4. 最終 render — production sink に emit
   render_streaming(&doc, defaults, resolver, StreamingConfig {
       lookahead, limits,
       initial_registry: prev,
   }, &mut production_sink)?;
```

**重要 (round 4 #2 対応)**: 各 iteration では **`plan()` のみを呼ぶ**。plan は
render を行わないので production sink に partial pages が emit される問題は
発生しない (§3 で明示済み: plan は "PaintScene / PaintedBox 構築なし")。最終
`render_streaming` のみが本 sink を触る。

**なぜ収束するか**:
- `plan()` が `initial_registry` を hint として受け取り、layout はその hint 値の
  幅を使って spacing (round 4 #2 対応で `PlanConfig` に追加)
- Iteration `N` は iteration `N-1` の target 値を仮定して layout
- 収束: 次 iter で hint と actual が一致 → registry は変わらない
- 発散するケース (幅が桁上がりで振動する場合等) は max_iterations で打ち切り、
  最後の registry を production render に使用

**intermediate render を実 sink に出さない protocol**:
- 各 iteration は `plan()` のみで render_* を呼ばない (実 sink 未使用)
- 最終 iteration のみ `render_streaming` を呼び production sink に出す

**fulgur config example**:

```rust
struct FulgurRenderConfig {
    /// public request (untrusted): 0 or 1、trusted batch: 3-5
    max_target_iterations: u32,
    iteration_timeout: Duration,
    total_timeout: Duration,
    max_memory: usize,
}

// Consumer iteration (fulgur 側)
let mut prev_registry: Option<TargetRegistry> = None;
let doc = raikiri::parse_html(input, &options)?;  // parse は 1 度で済む
for _iter in 0..fulgur_config.max_target_iterations {
    let plan = raikiri::plan(&doc, defaults, &resolver, PlanConfig {
        lookahead, limits: RenderLimits::default(),
        initial_registry: prev_registry.clone(),
    })?;
    if Some(&plan.target_registry) == prev_registry.as_ref() {
        // 収束: このplanで確定した registry を使って本 render
        break;
    }
    prev_registry = Some(plan.target_registry);
    // fulgur の判断で abort 可能 (時間、メモリ、request tier 等)
    if fulgur_should_abort() { break; }
}

// pass-N+1: 本 render (同じ &Document を使う)
raikiri::render_streaming(&doc, defaults, &resolver,
    StreamingConfig {
        lookahead: LookaheadConfig::default(),
        initial_registry: prev_registry,  // 収束済み registry を hint に
    },
    &mut sink)?;
```

**DoS 耐性**: attacker が iteration を無制限に強制することはできない。fulgur の
config が上限を設定し、iteration 中でも Consumer が中断可能。

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

### 8.2 差分の一覧 (strategy 化される部分のみ、+ plan mode)

| 項目 | `plan` (dry-run) | Streaming プリセット | Batch プリセット |
|---|---|---|---|
| **LookaheadPolicy** | 通常 `UnboundedLookahead` (精度重視) | `BoundedLookahead(cfg)` | `UnboundedLookahead` |
| **TargetResolver** | 内部で target 収集 | `PlaceholderTargetResolver` | `RegistryTargetResolver` (initial_registry の hint 使用) |
| **EmissionPolicy** | 出力なし (PaintedBox 構築せず) | `ImmediateEmission` | `DeferredEmission` |
| **ReflowPolicy** | `AggressiveCommit` | `AggressiveCommit` | `AggressiveCommit` (M1〜M8) / `FullReflow` (post-M8) |
| **target-* 挙動** | 内部で全 target 解決 → DocumentPlan.target_registry (hint) | 未解決は placeholder emit、hint あれば hint 値 (実測で上書き) | hint あれば hint 値、実測との差は discrepancies に記録 |
| **container probe** | 上限なし | 上限 N ページ、超えたら `ContainerOverflowFallback` | 上限なし、Fragmentation L3 準拠 |
| **iteration** | 1 pass 固定 | 1 pass 固定 | 1 pass 固定 (Consumer が chain して iteration) |
| **Memory** | `O(DOM + page_summary)`  | `O(DOM + window + 1 page)` | `O(DOM + all pages)` |
| **CSS 挙動** | render と同一 | Batch と同一 (Phase A で完結) | Streaming と同一 (Phase A で完結) |
| **PaintedBox tree** | なし (dry-run) | 有り (PageFragment) | 有り (PageFragment) |

**重要な訂正 (Finding #1 / #7 対応)**: 前版で挙げていた「Streaming では `:has()`
不可、Batch のみ対応」等の差は**存在しない**。それらは Phase A の cascade で
mode 非依存に解決されるため、mode 選択は cascade の挙動に影響しない。

**重要な訂正 (Finding #5 対応)**: 前版で挙げていた「Batch = full spec compliance
+ NPassConverge 収束保証」は**廃止**。DoS 耐性のため raikiri は 1 pass 固定、
target-* 収束が必要な用途は Consumer が `plan` + `render_*` を chain
して自分の iteration bound で管理する。

### 8.3 mode 選択 API

```rust
// pre-composed entry (通常 Consumer 向け、new review #5 対応で &Document 前提)
// round 4 review #1, #2 対応: PlanConfig で limits と initial_registry 受け取り
pub fn plan(
    doc: &Document,
    defaults: PageDefaults,
    resolver: &dyn ReplacedResolver,
    config: PlanConfig,
) -> Result<DocumentPlan, RenderError>;

pub fn render_streaming(
    doc: &Document,
    defaults: PageDefaults,
    resolver: &dyn ReplacedResolver,
    config: StreamingConfig,   // Finding #5: hint 経由の initial_registry
    sink: &mut dyn RenderSink,
) -> Result<RenderStatus, RenderError>;

pub fn render_batch(
    doc: &Document,
    defaults: PageDefaults,
    resolver: &dyn ReplacedResolver,
    config: BatchConfig,       // Finding #5: initial_registry あり
    sink: &mut dyn RenderSink,
) -> Result<RenderStatus, RenderError>;

// advanced: 任意の strategy 組み合わせ
pub fn render_with<L, T, E, R>(
    doc: &Document,
    defaults: PageDefaults,
    resolver: &dyn ReplacedResolver,
    sink: &mut dyn RenderSink,
    lookahead: L, target: T, emission: E, reflow: R,
) -> Result<RenderStatus, RenderError>
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
target-* が page number を変えるケースで、それは Consumer が `plan` +
`render_*` を chain して自分で管理する (§7.4 参照、raikiri 内 iteration は
Finding #5 対応で廃止)。

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

**副次効果 (訂正、Finding #6 対応)**: sync resolver は "resolver 起動時点の attrs
を参照する契約" を保証。これは **pre-sanitize な captured URL が resolver 側に残る
問題** を防ぐ。ただし、**scheme 検証 (`javascript:` 拒否)、size 制限、MIME 検証、
timeout、redirect 制御、decompression bomb 対策 等の resource-level security は
sync resolver だけでは実現できない** — これらは §10.1 で扱う `ResourcePolicy` +
`SandboxedResolver` / `SandboxedNetProvider` (raikiri-net) の責務。

### 10.1 Security の 2 層 model (Finding #6 対応)

raikiri は **Consumer が 2 層 security を実装する必要があること** を明示化:

```
Layer 1: DOM level sanitize (§5.2)
  - TreeSink wrap で <script>, <iframe>, on* attr 削除
  - `javascript:` scheme を href/src から除去
  - untrusted HTML の "パースする内容" の防御

Layer 2: Resource level policy (§10, raikiri-net)
  - SandboxedNetProvider + SandboxedResolver で fetch 時に policy 適用
  - ResourcePolicy: scheme allowlist、host restriction、size limit、
    MIME validation、timeout、redirect 制御、recursion limit
  - untrusted resource の "fetch する対象" の防御
```

**両方の Layer を Consumer が実装しないと server-side safety は成立しない**。
1 層だけでは以下のような攻撃を防げない:
- Layer 1 のみ: 巨大画像 URL による memory 枯渇、遅い server への fetch による
  thread hang、redirect chain による資源浪費
- Layer 2 のみ: `<script>` の残存、`javascript:` scheme の残存、event handler
  の残存

### 10.2 ResourcePolicy trait と wrapper pattern

**trait 定義** (raikiri-traits、§4 参照)、**preset 実装** (raikiri-net):
- `DenyAllPolicy` — 全 fetch を reject する最も restrictive な起点
- `DefaultSandboxPolicy` — https + data のみ許可、10 MB 上限、5s timeout 等の実用的デフォルト

**wrapper pattern**:

```rust
// Consumer's own base provider (raw、policy なし)
struct FulgurNetProvider { /* ... */ }
impl NetworkProvider for FulgurNetProvider { /* ... */ }

// Fulgur が 2 種類の provider を用意:
// (A) trusted internal batch job 用: 生 provider
let np_trusted = FulgurNetProvider::new(assets_dir);

// (B) public API / untrusted input 用: Sandboxed でwrap
let policy = Arc::new(DefaultSandboxPolicy::default());
let np_untrusted = SandboxedNetProvider::new(
    FulgurNetProvider::new(assets_dir),
    policy.clone(),
);
let resolver_untrusted = SandboxedResolver::new(
    FulgurResolver::new(font_data, images),
    policy,
);
```

**利点**:
- Signature が blitz と shape 一致 — Consumer が blitz と両対応する場合の adapter
  が薄くて済む
- Consumer の既存 impl は breaking change なし
- `LoggingProvider::new(SandboxedNetProvider::new(FulgurNetProvider::new()))` の
  ような chain 合成が natural

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

### 12.4 Blitz baseline oracle (訂正版、new review #3 対応)

dev-dep として blitz を使い、同じ WPT test に対する blitz と raikiri の pass/fail
状況を diff として記録する **T3 (informational) の tool**。以前の「blitz pass =
raikiri 必須 pass」記述は撤回。

**tier 別の扱い**:
- **T3 記録**: blitz と raikiri の pass 差分を nightly で集計。blitz-only pass
  は「raikiri が今後対応する候補」の情報として提供、blocking ではない
- **T2 promotion**: 特定の blitz-only pass を「raikiri baseline に追加」と
  明示的に決定した場合のみ T2 に昇格し、regression detection の対象に
- **raikiri baseline** (`expectations/raikiri-baseline.txt`): raikiri が現在
  pass している集合。この集合の pass → fail 変化のみ block

つまり、blitz oracle は「raikiri のここが blitz より遅れている」の signal だが、
遅れていることそのものは block しない (低カバレッジでも実用的なら ship する
方針 §2 と整合)。

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

### 12.6 CI 頻度と blocking policy

| テスト | Tier | 頻度 | 時間 | 遮断性 |
|---|---|---|---|---|
| Unit + Integration | T2 | PR ごと | 数秒〜数分 | **必須** |
| **Reference documents VRT** | **T1 (ship 判定)** | PR ごと | 5〜10 分 | **必須** |
| WPT reftest (tracked subset) | T2/T3 | PR ごと | 15〜30 分 | **必須** (regression のみ) |
| Byte-identical (10 回実行) | T2 | PR ごと | 数十秒 | **必須** |
| Preset-independence | T2 | PR ごと | 数分 | **必須** |
| WPT full sweep | T3 | nightly | 1〜2 時間 | 記録 (pass rate trend) |
| Blitz baseline oracle diff | T3 | nightly + weekly | 数時間 | 記録 (blitz との delta) |
| PDF reftest (via fulgur adapter) | T1/T2 | nightly (M6 以降) | 1〜2 時間 | **必須** (M6 以降) |
| Cross-platform tier 2 raster | T3 | nightly | 数十分 | 記録 |

**Tier の意味 (Finding #9 対応)**:
- **T1 (ship 判定基準)**: fulgur が render する代表 reference documents。visual
  一致が ship の必要条件
- **T2 (品質維持)**: 既 pass テストの regression 防止。scope 内外 flat rule
- **T3 (進捗追跡)**: WPT pass rate、blitz oracle delta、cross-platform raster
  等を記録し trend を観察。絶対値の目標なし

### 12.7 Reference documents (Finding #9 対応、T1 の詳細)

`tests/reference/` 配下に fulgur ユースケース由来の代表ドキュメントを配置:

```
tests/reference/
├── invoice-en/         # 英字圏インボイス、target-* あり、複数ページ
├── report-jp/          # 日本語レポート、running header、TOC
├── contract-mixed/     # 契約書、named page、mixed size、bleed
├── certificate/        # 証明書 (1 ページ、日付+署名エリア)
├── technical-book/     # 技術書 (章立て、counter、footnote、index)
├── invitation-a5/      # A5 招待状
├── manual-multicol/    # マニュアル (2 column)
└── ...
```

各ディレクトリの構成:
- `input.html`
- `expected/page-{N:04}.png` (VRT reference、per-page、tier 1 platform で生成、
  詳細は下記「Multi-page ドキュメントの golden 表現」)
- `expected.pdf` (fulgur adapter 経由、M6+ で)
- `expected-summary.json` (RenderSummary の canonical serialize)
- `README.md` (このドキュメントが検証している要件)

**追加時のルール**: 新 milestone で「この milestone で対応する新機能」に対応する
reference document を最低 1 つ追加。既存 reference は regression させない。

**Multi-page ドキュメントの golden 表現** (new review #6 対応):
- 各 reference directory は `expected/` サブディレクトリを持ち、per-page で
  `page-{N:04}.png` (N は 0-indexed) を配置
- 例: `tests/reference/report-jp/expected/page-0000.png`, `page-0001.png`, ...
- 加えて `expected.pdf` (M6 以降、fulgur adapter 経由の end-to-end)
- 加えて `expected-summary.json` (RenderSummary の canonical serialize)

**VRT pixel tolerance** (new review #6 対応):
- **Tier 1 (Linux x86_64)**: pixel-exact (tolerance = 0)
- **Tier 2 (Linux aarch64, macOS)**: max_delta = 1 per channel, max_diff_pixels =
  0.1% of total
- **Tier 3 (Windows)**: max_delta = 2 per channel, max_diff_pixels = 0.5%

**Golden 更新 process** (new review #6 対応):
1. Consumer (developer) が意図的な変更で expected が変わる場合、`cargo test --
   --update-goldens` で新 expected を生成
2. `git diff` で差分を確認、diff visualization (`expected-diff.png`) を CI
   artifact として自動生成
3. PR に diff visualization + 変更理由を明記
4. Reviewer は視覚的に承認、承認後 merge
5. Automatic 更新は禁止 (accidentally regression を防ぐ)

### 12.8 Byte-identical scope の spec

**比較対象** (優先度順):
1. `PageFragment` stream の serialize (JSON schema 固定、field 順 sorted)
2. `PaintCommand` stream (via `anyrender_serialize`)
3. Rasterized PNG (via `raikiri-vrt` = `anyrender_vello_cpu`、DPI 固定)

**Platform matrix**:
- **Tier 1** (byte-identical 保証): Linux x86_64
- **Tier 2** (structural + tolerance rasterize): Linux aarch64, macOS x86_64/aarch64
- **Tier 3** (best effort): Windows x86_64

**固定 dep version** (Cargo.lock を Git commit で管理):
- `anyrender_vello_cpu`, `parley`, `taffy`, `tiny-skia`, `selectors`, `cssparser`
  を`=X.Y.Z` prefix で pin
- workspace の Cargo.toml で明示

**Font 選定 (訂正版、Finding #9 対応)**:
- **WPT reftest**: WPT submodule の bundled fonts のみ (`wpt/fonts/*` — Ahem、
  DejaVu、Noto の必要分。WPT が定めるバージョン)
- **Reference documents (T1)**: Ahem を優先 (glyph metrics 予測可能で
  byte-identical に強い)。日本語ドキュメントは WPT の
  `css/css-writing-modes/support/` にある test font を使用、必要なら小 fixture 追加
- **submodule commit hash で font 版を pin**: WPT submodule の commit hash を
  fixed、raikiri が独自 bump しない

**Float 正規化** (new review #6 対応、canonical serialization spec):
- 内部型: `f32` (座標、色、寸法) と `f64` (計算過渡値) が混在
- **canonical serialization での正規化**:
  - 全て `f64` に昇格 (`x as f64`) してから正規化
  - Round-half-even で 6 decimal places に量子化 (`(x * 1e6).round() / 1e6`)
  - **Negative zero → Positive zero** (`if x == 0.0 { 0.0 } else { x }`)
  - **NaN → error**: 妥当な layout output に NaN は現れない前提、検出時は
    `RenderError::Layout(LayoutError::NonFiniteFloat)` で fail-fast
  - **Infinity → error**: 同上、`NonFiniteFloat` で fail-fast
- **format**: `format!("{:.6}", normalized_value)` で string 化して hash / JSON
  出力

**Map 順序**: `BTreeMap` or `IndexMap` を使用、`HashMap` は byte-identical
出力に使わない (iteration 順が非決定的)

#### Determinism testing matrix (round 3 Missing #3 対応)

byte-identical goal を複数次元で verify:

| 次元 | 対象 | Tier | 保証レベル |
|---|---|---|---|
| **Rayon thread count** | 1, 2, 4, 8, 16 | T1/T2 | pixel-exact byte-identical (全 tier) |
| **Process** (同一 platform で複数プロセス) | 10 processes | T1 | byte-identical |
| **Architecture** | x86_64, aarch64 | T1 x86_64 は pixel-exact / T2 aarch64 は tolerance | 表 §12.8 参照 |
| **OS** | Linux, macOS, Windows | Linux T1, macOS T2, Windows T3 | 同上 |
| **Font environments** | system fonts / bundled fonts / WPT fonts | 全 tier で bundled/WPT のみ使用 | pixel-exact |
| **Dep upgrade** | Cargo.lock 更新後の差分 | patch bump = 保証、minor bump = re-verify | version bump 時に手動 verify |

**T1 Determinism baseline (Linux x86_64)**:
- Rayon thread count を 1〜16 で回して 100% pixel-exact
- 同一 platform で 100 processes を回して 100% pixel-exact
- Font は WPT bundled + Ahem のみ
- Dep は Cargo.lock pin

**T2 (Linux aarch64、macOS x86_64/aarch64)**:
- pixel tolerance = 1 per channel、diff pixel ratio ≤ 0.1%
- 差分 root cause: SIMD の丸め方向差、font rasterizer implementation 差
- Thread count / process 変化は T1 と同じ保証

**T3 (Windows x86_64)**:
- pixel tolerance = 2 per channel、diff pixel ratio ≤ 0.5%
- best effort、regression 追跡のみ

**Dep upgrade 時の determinism 保証**:
- patch bump (X.Y.Z → X.Y.Z+1) は SemVer 上 API 互換だが implementation
  change あり得るため、CI で verify
- minor bump (X.Y.Z → X.Y+1.0) は re-verify 必須、baseline 更新の PR
- major bump は full re-verify + PR review

**M8 の `cross-thread-cross-arch-cross-os-determinism-tests` task がこの matrix
全体を CI で verify**。個別 dimension の fail は matrix cell 単位で記録され、
regression detection の粒度が上がる。

### 12.9 WPT scope の位置付け (訂正版、Finding #9 対応)

WPT scope は「**pass すべき集合**」ではなく「**注視している領域**」として管理:

`expectations/tracked-wpt.txt`:
- 追跡している WPT カテゴリ / test id
- 各の pass 状況を記録するが、`pass rate = X% 必須` の threshold は設けない

`expectations/known-issues.txt`:
- 「pass できないが実用に影響なし」と判断した test をログ
- 例: `css/css-transitions/*` は non-goal で全 skip、というエントリを明示

**追跡対象カテゴリ** (representative、Non-exhaustive):

| WPT カテゴリ | 追跡優先度 | 補足 |
|---|---|---|
| `css/css-page/` | 高 | paged media 中核 |
| `css/css-fragmentation/` | 高 | break policy 中核 |
| `css/selectors/` | 高 | cascade 検証 |
| `css/css-text/` | 中 | text layout |
| `css/css-values/` | 中 | value spec |
| `css/css-writing-modes/` | 中 | MVP は horizontal のみだが parse 準拠 |
| `html/rendering/` | 中 | HTML rendering spec |
| `css/css-transforms/` | 低 | 3D transform は non-goal |
| `css/css-animations/` | 非追跡 | Non-goal (interactive) |
| `css/css-transitions/` | 非追跡 | Non-goal (interactive) |
| `html/interaction/` | 非追跡 | interactive rendering は Non-goal |

### 12.10 Flaky-test quarantine + baseline migration + exception 承認 (Missing consideration 対応)

**「every previous pass blocks」rule は厳しすぎる場面がある** (flaky test、
spec 変更、意図的な feature drop 等)。以下の process で例外を管理:

#### Quarantine list (`expectations/quarantine.txt`)

Flaky test の一時退避 (**platform-aware format**、round 3 review #7 対応):

```
# expectations/quarantine.txt
# format: test_id | platform | arch | renderer | tolerance | reason | issue_link | added_date
# 
# platform: linux / macos / windows / * (全 platform)
# arch: x86_64 / aarch64 / * (全 architecture)
# renderer: vello_cpu / skia / tiny_skia / * (全 renderer)
# tolerance: pixel-exact / low / medium / high / * (どの tolerance でも)
#   → 「quarantine を発火する tolerance 閾値」ではなく「どの tolerance CI で
#     flaky か」を記録するメタデータ
#
# 同じ test id を複数 platform 組み合わせで別 entries として書ける
css/css-page/page-margin-boxes-001 | macos | aarch64 | vello_cpu | pixel-exact | Intermittent 1-pixel diff | github.com/.../issues/123 | 2026-08-01
css/css-page/page-margin-boxes-001 | windows | x86_64 | * | * | 別 issue、renderer 非依存 | github.com/.../issues/124 | 2026-08-05
```

**マッチング**: CI 実行時の (platform, arch, renderer, tolerance) が entry の
filter 全て一致すれば quarantined、Linux x86_64 vello_cpu では上記例では
quarantine されず通常評価。

**Quarantine 手順**:
1. Test が 3 回以上 intermittent fail → 開発者が quarantine PR を出す
2. `quarantine.txt` にエントリ追加、issue を作成
3. CI は quarantined test を **T3 informational** として扱う (blocking 除外)
4. **週次 review**: 全 quarantined test を list、fix 完了なら quarantine 削除
5. 90 日以上 quarantine されたら strategic review (実装廃止 or fix priority up)

#### Baseline migration + Exception 承認 (統合、round 3 review #6 対応)

以前 baseline 追加 = 1 reviewer、exception = 2 reviewer と分けていたが、同じ
regression が両方に分類できる曖昧さを排除するため **統合し、全ての意図的な
pass 状態変化を 2 reviewer 必須** とする。

**Baseline migration (統一 process)**:

Test id を `raikiri-baseline.txt` に **追加 / 削除**する PR、および前 pass 状態
を意図的に fail 状態に変える PR は全て以下:

1. 開発者が PR で:
   - 該当 test id list
   - 変更方向 (add / remove / pass→fail)
   - 変更理由 (spec 準拠改善 / feature drop / test invalidated 等)
   - test を `expectations/` の他 file (deprecated / quarantine) に移動する
     場合はその明示
2. CI で該当 test の local 状態を verify
3. **2 名の reviewer 承認** (必須)
4. Merge 後、baseline が更新

**Automatic 更新は禁止**: すべての baseline / quarantine / deprecated 変更は
PR 経由。

#### Expectations files の precedence (round 3 review #6 対応)

同じ test id が複数 file に entries を持つ場合の precedence を明示:

| Precedence 順 | File | 意味 | CI 挙動 |
|---|---|---|---|
| 1 (最優先) | `expectations/deprecated.txt` | この test は評価対象外 | fully excluded (実行もしない) |
| 2 | `expectations/quarantine.txt` | flaky、informational | 実行するが結果は非 blocking |
| 3 | `expectations/raikiri-baseline.txt` | T2 gate、pass 必須 | 実行、fail なら PR block |
| 4 (デフォルト) | どこにもなし | tracking 対象外 | 実行するが結果は informational (T3) |

**Conflict handling**: 同一 test id が複数 file に登場 → CI が fail-fast で
「conflict detected in expectations」を報告、開発者が resolve する PR を出す。
Automated validation task (`validate-expectations-files`) を Section 12 で追加。

#### Expectations validation の詳細 (round 3 Missing #4 対応)

`validate-expectations-files` task が CI で以下を検出、fail-fast:

**Malformed**:
- 列数が仕様と異なる (quarantine は 8 列、baseline は 1 列 = test_id のみ)
- 予期しない characters (空白 delimit の混在、trailing whitespace)
- Encoding が UTF-8 でない
- 各 `platform` / `arch` / `renderer` / `tolerance` が定義された enum に無い値

**Duplicate**:
- 同 file 内で同一 test_id + 同一 filter combination が複数 entries
  (ただし quarantine は複数 platform combinations で意図的に同 test_id が
  出現するので、重複判定は (test_id, platform, arch, renderer, tolerance) の
  tuple 単位)

**Expired**:
- Quarantine entry の `added_date` が 90 日超過、かつ referenced issue
  (`issue_link`) が closed → strategic review が必要 warning
- 削除は開発者判断だが warning で通知

**Conflicting** (§12.10 の precedence 表と組み合わせ):
- 同一 test_id が deprecated と baseline に同時 → conflict、優先度 rule に
  照らして解消 PR を要求 (deprecated が優先、baseline から削除する PR)
- 同一 test_id が quarantine と baseline に同時 → conflict、quarantine 側の
  filter が baseline の実行環境と重ならなければ OK、重なると conflict

Validation は `raikiri-wpt` crate の binary として実装 (`cargo run --bin
validate-expectations`)、CI で PR ごとに実行。

## 13. Milestone Plan

各 milestone は **Goals** (何を達成する) + **Tasks** (task 分解) +
**Acceptance criteria** (完了条件) + **Reference fixtures** (T1 gate) の 4 要素
で構成 (Task list findings #H2, #H3, #M4 対応)。

### M0: Dep feasibility + production workspace (round 3 review 対応)

**訂正 (round 3 review #3 対応)**: 以前「throwaway smoke」と書いたが実態と
矛盾。M0 は **production workspace の確立 + feasibility 検証** を両方担う:
- Cargo workspace scaffold、rust-toolchain、全 crate manifests は **production
  artifacts** (M1 以降も生き続ける)
- `crates/raikiri-feasibility/` の spike code のみ **disposable** (M0 完了時に
  削除するか、`examples/` に移動)

**M0 内部 DAG** (round 3 review #5 対応):
```
1. rust-toolchain-msrv          ← 独立
2. workspace-scaffold           ← 独立
   crate-manifests              ← 独立
3. dependency-resolution        ← 1,2 に依存 (Cargo.lock 生成)
4. 個別 spikes (parallel 可能)   ← 3 に依存
   - feasibility-parley
   - feasibility-taffy
   - feasibility-anyrender-vello-cpu (byte-identical raster verify)
   - feasibility-selectors
   - feasibility-cssparser
   - feasibility-selectors-cssparser-version-compat (round 3 #4)
   - feasibility-paintscene-adapter-compile-spike (round 3 #4)
5. feasibility-report           ← 全 spike に依存
6. m0-readiness-gate            ← report に依存、outcome 判定
```

**検証項目 (round 3 review #4 対応で完全化、全 7 項目)**:
- **parley**: `FontContext` の `Send + Sync` 適合性
- **taffy**: block/flex/grid、column-count 並列可能性
- **anyrender_vello_cpu**: 同一 platform で byte-identical raster
- **selectors**: standalone (`stylo` 非依存) 使用可能性
- **cssparser**: `@page` / `@counter-style` / `@font-face` の custom at-rule
  extension mechanism
- **selectors + cssparser 版整合** (round 3 #4 で明示 task 化): 両 crate を
  workspace で同 semver に pin 可能か
- **anyrender PaintScene 適合面** (round 3 #4 で明示 task 化): blitz-paint と
  shape 一致するか、adapter compile-spike で verify

**Tasks** (依存順、round 3 #5 対応):
- rust-toolchain-msrv (`rust-toolchain.toml` + MSRV = 1.85)
- workspace-scaffold (`Cargo.toml` workspace root、production)
- crate-manifests (全 crate `Cargo.toml`、pin dep version、production)
- dependency-resolution (`Cargo.lock` 生成、version compat verify)
- feasibility-parley, feasibility-taffy, feasibility-anyrender-vello-cpu,
  feasibility-selectors, feasibility-cssparser,
  feasibility-selectors-cssparser-version-compat,
  feasibility-paintscene-adapter-compile-spike
- feasibility-report (`docs/feasibility-report.md` に全結果、API surface 記録)
- **m0-readiness-gate**: M0 outcome 判定 (下記)

**M0 outcome の 2 分岐** (round 3 review #2 対応):
1. 全 7 検証項目で "OK" → M0 epic close、**M1 unblock**
2. 1 つでも "NEEDS_DESIGN_CHANGE" → 別 epic `raikiri-spike-m0-revision` を
   起動、design doc revise + roborev 再レビュー完了 → その後 M1 unblock

**M1 依存の precise 定義**: M1 は `m0-readiness-gate` task が「All checks OK」
判定を出した場合のみ start 可能。`NEEDS_DESIGN_CHANGE` の場合は m0-revision
epic の完了を追加で待つ。single `bd close raikiri-spike-m0` では M1 は自動 open
しない (readiness-gate の判定結果を beads の別 field で明示的に record)。

**Production 生存 artifacts** (M1 以降も残る):
- `Cargo.toml` (workspace root)
- `rust-toolchain.toml`
- 全 crate の `Cargo.toml`
- `Cargo.lock`
- `docs/feasibility-report.md`

**Disposable artifacts** (M1 開始前 or 開始時に削除):
- `crates/raikiri-feasibility/` の spike code
- `docs/feasibility-report.md` は残すが、spike crate は `examples/` に移動 or
  削除

**M1 の重複削除**: 以前 M1 tasks に `workspace-setup` があったが、M0 で完了
するので M1 から削除 (round 3 review #3 対応)。

**Non-goals**: 実際の HTML/CSS 処理 (M1 以降)、performance benchmarking
(M8 まで)、edge case exhaustive coverage (smoke test 相当)

### M1: Skeleton + reference harness + error taxonomy + hello world VRT

**Goals**:
- 全 crate skeleton (Cargo.toml, trait 定義, 最小型)
- end-to-end pipeline (parse → cascade minimal → layout single-page → paint → PNG)
- `RenderError` / `RenderStatus` type 定義と基本 error 伝播 (Parse/Cascade)
  (Task #H1 対応)
- Reference harness 初期セットアップ (`tests/reference/` の infrastructure)
  (Task #M4 対応)

**Tasks** (workspace-setup は M0 で完了、round 3 review #3 訂正):
- traits-definition, error-taxonomy-types
- html-parse-basic (Parse error), css-cascade-basic (Cascade error)
- dom-model, layout-single-page, paint-basic
- vrt-tiny-skia, wpt-harness-skeleton, reference-harness-scaffold
- umbrella-facade (parse_html, plan, render_streaming stubs)
- ci-setup, determinism-test, hello-world-vrt
- **public-api-compile-tests-lookaheadconfig** (round 3 review #2 対応、
  external consumer crate を模擬した compile test)

**Reference fixtures**:
- `tests/reference/hello-world/`: `<p style="color:red">Hi</p>` → PNG

**Acceptance criteria**:
- `cargo build` all crates green
- Unit test per crate (最低 1 個)
- `raikiri::html_to_png(HELLO)` returns Vec<u8>、hello-world VRT pass
- 10 回連続実行で byte-identical
- CI green
- 全 `#[non_exhaustive]` pub struct が external consumer crate から
  constructable (round 3 review #2 対応)
- **round 4 review Task #1 訂正**: BiDi、mixed Latin/CJK は M3 のみ。M1 は
  ASCII (Latin) 単一 script + 単一 font に限定

**Non-goals**: pagination、@page、GCPM、Batch、ReplacedResolver 実使用、
break policy、per-page PageBox、L4 selector、target-*、**BiDi、mixed script**
(round 4 review Task #1 対応、M3 に完全に移す)

### M2: Streaming pagination + LayoutBuffer + Streaming error semantics

**Goals**:
- 複数ページ縦流し、break-before/break-after 基本
- `LayoutBuffer` の skeleton
- **Streaming error semantics 実装** (partial output tracking、accept_page から
  Err) (Task #H1 対応)
- Streaming reference fixtures 追加

**Tasks**:
- pagestream-state-machine, layoutbuffer-skeleton
- break-before-after-forced (@page break-before)
- streaming-partial-output-tracking, sink-state-transition-tests
- accept_page-error-propagation, RenderStatus-integration
- reference-fixtures-multi-page

**Reference fixtures**:
- `tests/reference/simple-multi-page/`
- `tests/reference/break-before-basic/`

**Acceptance criteria**:
- 3-5 ページの縦流しが VRT で reference と一致
- `accept_page` が Err を返した時、上位が `RenderError::Sink` を受け取り、
  直前まで committed pages を確認可能
- Non-goal からの regression 無し (M1 hello world も pass)

### M3: Inline layout 本実装 (parley 統合)

**Goals**:
- parley 統合による text shape、BiDi、font selection
- 多国籍テキストの raster

**Tasks**:
- parley-integration, bidi-support, font-fallback
- inline-formatting-context, glyph-run-emit
- text-multilingual-fixtures

**Reference fixtures**:
- `tests/reference/text-multilingual/`
- `tests/reference/mixed-fonts/`

**Acceptance criteria**: BiDi text、混合 script (Latin + CJK) の raster が
reference と一致

### M4: @page rule cascade + per-page PageBox + margin box + GCPM state seed

**訂正 (Task list #M3 対応)**: per-page PageBox を M8 から **M4 に前倒し**。
理由: M5 の running template が per-page geometry を前提とするため。

**Goals**:
- @page 規則 cascade (`:first`, `:left`, `:right`, `:nth-page`, named)
- **per-page PageBox 決定 (mixed-size PDF 対応)** ← M8 から前倒し
- 16 margin box slot layout
- `PageContext` の seed 実装
- Layer 2 security (SandboxedNetProvider) の PolicyViolation 伝播 (Task #H1)

**Tasks**:
- page-rule-cascade, page-name-transition-table
- per-page-pagebox-resolver, pageboxcache-implementation
- margin-box-slot-layout, sixteen-slot-rayon
- pagecontext-seed-builder
- **Layer 2 security wrapper 実装** (round 4 review Task #4 対応、以前 unowned):
  - sandboxed-net-provider-impl (`SandboxedNetProvider<P>` wrapper)
  - sandboxed-resolver-impl (`SandboxedResolver<R>` wrapper)
  - denyall-policy-impl (`DenyAllPolicy`)
  - default-sandbox-policy-impl (`DefaultSandboxPolicy`)
  - policy-violation-propagation (raikiri-dom 内で violation を RenderError::Policy に伝播)
- **Layer 2 security tests** (round 4 review Task #4 対応):
  - ssrf-defense-tests (scheme allowlist、host restriction、private IP block)
  - resolver-parity-tests (SandboxedResolver が wrap する Consumer resolver と
    parity 動作、violation 時のみ diverge)
  - size-mime-timeout-enforcement-tests

**Reference fixtures**:
- `tests/reference/mixed-size-A4-A3/`
- `tests/reference/running-header-basic/` (static content only、mixed-size で検証)

**Acceptance criteria**:
- Mixed-size PDF (portrait A4 + landscape A3) が VRT で reference と一致
- Per-page PageBox 実装が §9.1 の遷移表通りに動作

### M5: GCPM directive full + running element (geometry-parameterized)

**訂正 (Task list #M3 対応)**: running template の geometry-parameterized test を
M5 に含める。M4 で導入された per-page PageBox に依存するので、順序は M4 → M5 で
安全。

**Goals**:
- counter-increment/reset/set、string-set 4-snapshot
- Running element template (M4 の per-page PageBox 上で動作)
- Batch preset skeleton (`UnboundedLookahead` の scaffold のみ、full 実装は M6b)

**Tasks**:
- gcpm-directive-emit, counter-tree-management
- string-set-4-snapshot
- running-template-store, per-page-relayout
- **running-template-replaced-content-preresolve** (round 3 review #5 対応):
  running template 内の replaced element を sequential phase で pre-resolve、
  結果を cache に格納する path
- **running-template-geometry-tests**: mixed-size で running 内容が正しく
  reflow されることを VRT + structural で検証
- **gcpm-snapshot-construction** (round 3 review #4 対応): `GcpmSnapshot` の
  owned deep copy semantics、interior mutability 禁止の実装 + lint 追加
- **thread-count-concurrency-tests** (round 3 review Task #1 対応): rayon の
  thread count (1, 2, 4, 8, 16) を変えて 16 margin box slot layout が全て
  byte-identical であることを verify
- batch-preset-skeleton

**Reference fixtures**:
- `tests/reference/chapter-counter/`
- `tests/reference/running-header-dynamic/` (counter/string 参照 + mixed-size)
- `tests/reference/running-header-with-logo/` (round 3 review #5 対応、
  running template 内 `<img>` の pre-resolve verify)

**Acceptance criteria**:
- 章立てレポート (counter-reset で chapter、counter-increment で section)
  が VRT で一致
- Running header が A4 と A3 の両ページで正しく reflow (geometry-parameterized
  test)
- Running template 内 `<img>` が pre-resolve され、margin box parallel layout
  で resolver 呼び出しがないことを verify (mock resolver で assertion)
- Rayon thread count 1〜16 で全 fixture が byte-identical

### M6: 5 sub-milestone に分割 (Task list #H2 対応)

**訂正**: 従来の 1 巨大 M6 (5000 行、極高) を独立 reviewable な 5 sub-milestone
に分割:

#### M6a: target-* SinglePass

**Goals**: `PlaceholderTargetResolver` 実装、`TargetSlotId` 安定化、
`target_definitions` per-page emit

**Tasks**: target-slot-emit, targetslotid-stability, target-definitions-per-page,
finish_render-summary-integration

**Reference fixtures**: `tests/reference/target-counter-basic/`

**Acceptance**: target-counter 参照が finish_render summary で完全 resolve、
Consumer patch できる

#### M6b: Batch preset 完成 (round 4 review Task #3 訂正)

**Goals**: `UnboundedLookahead` full 実装、`RegistryTargetResolver`、
`initial_registry` hint 対応、DeferredEmission
**訂正**: Fragmentation L3 の実装は M7 に完全に移す。M6b は preset の
plumbing のみ担当

**Tasks**: unbounded-lookahead-impl, registry-target-resolver,
initial-registry-plumbing, deferred-emission-impl,
batch-vs-streaming-preset-independence-test

**Reference fixtures**: `tests/reference/batch-preset-basic/`
(target-* を含む単一ページ + Deferred emission の smoke、multi-page flex/grid は
M7 に譲る)

**Acceptance** (round 4 review Task #3 訂正):
- `render_batch` が deferred emission で全 pages を最後にまとめて emit
- `initial_registry` hint が cache lookup で target-* を resolve
- Preset-independence test で `render_streaming` と `render_batch` が
  target-* 未使用ドキュメントで byte-identical
- **Fragmentation L3 acceptance は M7 (widow/orphan、break-inside、
  container probe が完成した後)**

#### M6c: Batch error semantics + Abort handling

**Goals** (Task #H1 対応): Batch の pre-emission vs post-emission-start 分岐、
`RenderStatus::Aborted` 実装、`AbortSignal` 統合

**Tasks**: batch-preemission-atomicity, batch-postemission-partial,
abort-signal-integration, renderstatus-aborted-impl, error-injection-tests-batch

**Reference fixtures**: `tests/reference/abort-mid-batch/` (structural test)

**Acceptance**: `AbortSignal.abort()` が graceful stop を発火、partial_pages を
正しく通知。error injection で全 variant が正しく伝播

#### M6d: fulgur adapter

**Goals**: fulgur の既存 blitz_adapter.rs 相当を raikiri 経由に置換する adapter

**Tasks**: fulgur-adapter-scaffold, fulgur-resolver-integration,
fulgur-sink-integration, fulgur-pdf-reftest-integration

**Reference fixtures**: `tests/reference/fulgur-integration-A/` (fulgur adapter
経由の end-to-end PDF)

**Acceptance**: fulgur の代表 fixture が raikiri adapter 経由で PDF 生成、
既存 blitz 版と visual 一致

#### M6e: raikiri-blitz-compat 初版

**Goals**: 方針 B (型 shape 互換、振る舞い raikiri) の初版

**Tasks**: blitz-compat-type-shape, blitz-html-htmldocument-compat,
blitz-dom-node-compat, behavior-diff-md

**Reference fixtures**: なし (compat 検証は compile-level test で、`tests/compat/`
に fulgur のスニペットを compile 通す test)

**Acceptance**: fulgur が `use raikiri_blitz_compat::blitz_html::HtmlDocument;` に
差し替えるだけで既存コードが compile 通る

### M7: Break policy + widow/orphan + probe layout + Abort integration

**Goals**:
- widow/orphan、break-inside: avoid の実装
- Container probe layout (§5.1 の probing フェーズ)
- `AbortSignal` を全 loop に統合 (M6c で骨格実装、M7 で徹底)

**Tasks**: widow-orphan-lookahead, break-inside-avoid-subtree-scan,
container-probe-layout, aggressivecommit-fallback,
abort-integration-comprehensive

**Reference fixtures**:
- `tests/reference/widow-orphan-A/`
- `tests/reference/break-inside-avoid/`

**Acceptance**: widow/orphan 制約が正しく効く、multi-page flex/grid で probe
layout が動作

### M8: 決定論 stress + WPT full sweep + Error injection completeness + Cross-platform

**Goals**:
- byte-identical CI stress (100 runs)
- Cross-platform tier 2/3 raster nightly
- WPT full sweep nightly + Blitz oracle diff
- **全 `RenderError` variant の failure-injection test 完備** (Task #H1)
- Golden update process demo と documentation

**Tasks** (round 3 review Task #1 対応で expanded):
- determinism-stress-test, cross-platform-raster-ci
- wpt-full-sweep-scheduler, blitz-oracle-diff-recorder
- error-injection-suite-completeness, golden-update-workflow-doc
- **quarantine-baseline-ci-processing** (round 3 review Task #1、#6 対応):
  `expectations/` の 4 file (baseline / quarantine / deprecated / expected)
  を CI が読み込み、precedence を適用、conflict detection、platform filter を
  適用する processing
- **validate-expectations-files** (round 3 review #6 対応、conflict handling):
  duplicate / expired / conflicting entry の automated validation。CI で
  fail-fast
- **cross-thread-cross-arch-cross-os-determinism-tests** (round 3 review Task
  #1、Missing 対応): rayon thread count / architecture / OS を組み合わせた
  determinism matrix
- **aggregate-budget-tests** (round 3 review Task #1 対応): 大 template ×
  多 page の性能 characterization test (Consumer 責任の再確認、raikiri は
  fail-fast しない)

**Reference fixtures**: 全既存 fixture を multi-platform で回す、
`tests/reference/error-injection-suite/` (每 RenderError variant 用の structural
test)

**Acceptance**:
- byte-identical 100 runs stress で 100/100 一致 (Tier 1)
- Tier 2 raster tolerance 内、Tier 3 best effort
- 全 `RenderError` variant を fault injection でトリガー可能
- Rayon thread count 1/2/4/8/16 で全 fixture が byte-identical (round 3 review
  Missing 対応)
- Expectations file の conflict / duplicate / expired を CI が検知して fail

### milestone 依存 DAG (訂正版、M0 gate 明示 + per-page PageBox 前倒し + M6 分割)

```
M0 dep feasibility + production workspace
     │
     ▼
m0-readiness-gate: outcome 判定
     │
     ├─ outcome = OK ──────────────────────────────┐
     │                                              │
     └─ outcome = NEEDS_DESIGN_CHANGE               │
           │                                        │
           ▼                                        │
        M0-revision: design revise + roborev        │
           │                                        │
           ▼                                        │
        m0-revision-close-gate                      │
           │                                        │
           └────────────────────────────────────────┤
                                                    ▼
M1 skeleton + reference harness + error taxonomy + hello world
     │
     ├─▶ M2 pagination + Streaming error semantics
     │      │
     │      ├─▶ M3 inline (parley 統合)
     │      │       │
     │      │       ▼
     │      │   M4 @page cascade + per-page PageBox + margin box + Policy propagation
     │      │       │
     │      │       ▼
     │      │   M5 GCPM directive full + running (per-page geometry) + Batch skeleton
     │      │       │
     │      │       ├───────────────┐   ┌── M6e blitz-compat (M5 依存のみ、並列可能)
     │      │       ▼               ▼   │
     │      │   M6a target-*    M6b Batch preset  (parallel、共有 interface が M5 に確定済)
     │      │   SinglePass           │
     │      │       │                │
     │      │       └────────┬───────┘
     │      │                ▼
     │      │            M6c Batch error + Abort handling (M6a, M6b の両方に依存)
     │      │                │
     │      │                ▼
     │      │            M6d fulgur adapter (M6a-c に依存、M6e にも依存)
     │      │                │
     │      └───────────────▶ M7 break policy + probe layout + Fragmentation L3 acceptance
     │                                  │
     │                                  ▼
     │                              M8 決定論 stress + WPT full sweep +
     │                              Error injection completeness + Cross-platform
     │                                  │
     └───────── T1 reference fixtures ──┘ (各 milestone で追加)
```

### サイズ感 (LoC 目安、M0 追加 + M6 分割済み)

| Milestone | 実装量目安 | 難易度 |
|---|---|---|
| M0 dep feasibility spike | ~500 行 (throwaway smoke) | 中 |
| M1 skeleton + reference harness + error types | ~2500 行 | 中 |
| M2 pagination + Streaming error semantics | ~2000 行 | 中 |
| M3 inline (parley) | ~3000 行 | 高 |
| M4 @page + per-page PageBox + margin box + Policy | ~3000 行 | 高 |
| M5 GCPM full + running (geometry) + Batch skeleton | ~3500 行 | 高 |
| M6a target-* SinglePass | ~1200 行 | 高 |
| M6b Batch preset | ~1500 行 | 高 |
| M6c Batch error + Abort | ~1000 行 | 高 |
| M6d fulgur adapter | ~1500 行 | 高 |
| M6e raikiri-blitz-compat 初版 | ~1000 行 | 中 |
| M7 break policy + probe + Abort integration | ~2500 行 | 高 |
| M8 決定論 + WPT + Error injection + cross-platform | ~2000 行 + fixture | 中 |
| **合計** | ~24,700 行 (既存 raikiri 16k + 8.7k 増) | — |

各 M6 sub-milestone は独立 reviewable (~1000-1500 行) で、PR review の負荷が
大幅に下がる (Task #H2 対応)。

### beads 構成 (訂正版、全 milestone task 分解済み)

各 milestone は epic として立て、上記 Tasks を task として `bd create` し、
`bd dep add` で milestone 間依存を明示。

```
raikiri-spike-m0 (epic): Dep feasibility + production workspace
├─ rust-toolchain-msrv         (leaf、no deps)
├─ workspace-scaffold          (leaf、no deps)
├─ crate-manifests             (leaf、no deps)
├─ dependency-resolution       (needs: workspace + manifests + toolchain)
├─ feasibility-parley                             (needs: dep-resolution)
├─ feasibility-taffy                              (needs: dep-resolution)
├─ feasibility-anyrender-vello-cpu                (needs: dep-resolution)
├─ feasibility-selectors                          (needs: dep-resolution)
├─ feasibility-cssparser                          (needs: dep-resolution)
├─ feasibility-selectors-cssparser-version-compat (needs: dep-resolution)
├─ feasibility-paintscene-adapter-compile-spike   (needs: dep-resolution)
├─ feasibility-report          (needs: 全 feasibility-* 完了)
└─ m0-readiness-gate           (needs: report、outcome = OK / NEEDS_DESIGN_CHANGE)

raikiri-spike-m0-revision (epic、M0 outcome = NEEDS_DESIGN_CHANGE の場合のみ起動):
├─ design-doc-revision
├─ roborev-re-review
└─ m0-revision-close-gate (承認 → M1 unblock)

raikiri-spike-m1 (epic): Skeleton + reference harness + error taxonomy
├─ traits-definition, error-taxonomy-types  (workspace-setup は M0 で完了)
├─ html-parse-basic, css-cascade-basic, dom-model
├─ layout-single-page, paint-basic
├─ vrt-tiny-skia, wpt-harness-skeleton, reference-harness-scaffold
├─ umbrella-facade, ci-setup, determinism-test
├─ public-api-compile-tests-lookaheadconfig  ★ round 3 review #2 対応
└─ hello-world-vrt (上記全てに依存)

raikiri-spike-m2 (epic): Streaming pagination + error semantics
├─ pagestream-state-machine, layoutbuffer-skeleton
├─ break-before-after-forced
├─ streaming-partial-output-tracking, sink-state-transition-tests
├─ accept_page-error-propagation, RenderStatus-integration
└─ reference-fixtures-multi-page

raikiri-spike-m3 (epic): Inline layout
├─ parley-integration, bidi-support, font-fallback
├─ inline-formatting-context, glyph-run-emit
└─ text-multilingual-fixtures

raikiri-spike-m4 (epic): @page + per-page PageBox + margin box + Policy
├─ page-rule-cascade, page-name-transition-table
├─ per-page-pagebox-resolver, pageboxcache-implementation
├─ margin-box-slot-layout, sixteen-slot-rayon
├─ pagecontext-seed-builder
└─ policy-violation-propagation

raikiri-spike-m5 (epic): GCPM directive + running (per-page geometry)
├─ gcpm-directive-emit, counter-tree-management
├─ string-set-4-snapshot
├─ running-template-store, per-page-relayout
├─ running-template-replaced-content-preresolve   ★ round 3 review #5
├─ running-template-geometry-tests   ★ mixed-size で running 検証
├─ gcpm-snapshot-construction   ★ round 3 review #4 (owned deep copy semantics)
├─ thread-count-concurrency-tests   ★ round 3 review Task #1
└─ batch-preset-skeleton

raikiri-spike-m6a (epic): target-* SinglePass
├─ target-slot-emit, targetslotid-stability
├─ target-definitions-per-page
└─ finish_render-summary-integration

raikiri-spike-m6b (epic): Batch preset
├─ unbounded-lookahead-impl, registry-target-resolver
├─ initial-registry-plumbing
└─ batch-vs-streaming-preset-independence-test

raikiri-spike-m6c (epic): Batch error semantics + Abort
├─ batch-preemission-atomicity, batch-postemission-partial
├─ abort-signal-integration, renderstatus-aborted-impl
└─ error-injection-tests-batch

raikiri-spike-m6d (epic): fulgur adapter
├─ fulgur-adapter-scaffold, fulgur-resolver-integration
├─ fulgur-sink-integration
└─ fulgur-pdf-reftest-integration

raikiri-spike-m6e (epic): raikiri-blitz-compat 初版
├─ blitz-compat-type-shape, blitz-html-htmldocument-compat
├─ blitz-dom-node-compat
└─ behavior-diff-md

raikiri-spike-m7 (epic): Break policy + probe layout
├─ widow-orphan-lookahead, break-inside-avoid-subtree-scan
├─ container-probe-layout, aggressivecommit-fallback
└─ abort-integration-comprehensive

raikiri-spike-m8 (epic): 決定論 + WPT + Error injection + Cross-platform
├─ determinism-stress-test (100 runs)
├─ cross-platform-raster-ci (tier 2/3)
├─ wpt-full-sweep-scheduler, blitz-oracle-diff-recorder
├─ error-injection-suite-completeness (全 RenderError variant)
├─ quarantine-baseline-ci-processing   ★ round 3 review #6, Task #1
├─ validate-expectations-files   ★ round 3 review #6 (conflict detection)
├─ cross-thread-cross-arch-cross-os-determinism-tests   ★ round 3 Missing
├─ aggregate-budget-tests   ★ round 3 review Task #1
└─ golden-update-workflow-doc
```

各 milestone epic は完了時に `bd close`。sub-milestone 間の依存 (round 4 review
Task #2 訂正: `M6a || M6b || M6e` parallel → `M6c` (depends on M6a + M6b) →
`M6d` (depends on M6a-c and M6e); prose was `M6a→M6b→M6c→M6d→M6e` linear、DAG は
parallel。DAG を採用、prose を訂正) は `bd dep add` で明示。

fixture 追加 task は milestone 各 epic 内に含める (Task #M4 対応)。

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

**Finding #5 対応後の推奨フロー**: `plan` で dry-run → DoS 判定 →
`render_*` で本描画。fulgur の既存 pass-1 / pass-2 アーキテクチャに直接対応。

```rust
use raikiri::{
    PageDefaults, LookaheadConfig, StreamingConfig, BatchConfig,
    RenderSink, ReplacedResolver, NetworkProvider,
    ParseOptions, RenderStatus, RenderError,
    parse_html, plan, render_streaming, render_batch,
};
use raikiri_net::{SandboxedNetProvider, SandboxedResolver};
use std::sync::Arc;

// 1. options を組む (template 展開後の HTML を含む)
//    Layer 2 security (Finding #6 対応): request tier で provider / resolver を選択
let (net_provider, resolver): (Box<dyn NetworkProvider>, Box<dyn ReplacedResolver>) = 
    match fulgur_config.trust_tier {
        TrustTier::PublicApi => {
            // untrusted input: Sandboxed で wrap
            let policy = Arc::new(fulgur::FulgurStandardPolicy::default());
            (
                Box::new(SandboxedNetProvider::new(
                    FulgurNetProvider::new(&assets_dir),
                    policy.clone(),
                )),
                Box::new(SandboxedResolver::new(
                    FulgurResolver::new(&font_data, &images),
                    policy,
                )),
            )
        }
        TrustTier::InternalBatch => {
            // trusted: 生 provider (性能重視)
            (
                Box::new(FulgurNetProvider::new(&assets_dir)),
                Box::new(FulgurResolver::new(&font_data, &images)),
            )
        }
    };

let options = ParseOptions {
    extra_stylesheets: &fulgur_stylesheets,
    network: Some(net_provider.as_ref()),
    base_url: Some(base),
};

let defaults = PageDefaults::from_cli(cli_size, cli_orientation);

// 2. pass-0: plan で document 全体像を掴む (DoS 防御ゲート)
// parse を 1 度だけ (new review #5 対応、input 再利用回避)
let doc = raikiri::parse_html(html_input, &options)?;

let plan = raikiri::plan(&doc, defaults, resolver.as_ref(), LookaheadConfig::default())?;

if plan.total_pages > fulgur_config.max_pages_per_request {
    return Err(FulgurError::DocumentTooLarge { pages: plan.total_pages });
}
if plan.unresolved_targets.len() > fulgur_config.max_unresolved_targets {
    return Err(FulgurError::TooManyUnresolvedTargets);
}

// 3. サイズと Fragmentation 要件で render_streaming / render_batch を選ぶ
let mut sink = FulgurPdfSink::new(krilla_doc);
let status = match plan.total_pages {
    n if n < 100 && has_multi_page_flex_grid(&plan) => {
        // Batch: Fragmentation L3 準拠が必要な小〜中規模文書
        // round 4 review #5: #[non_exhaustive] struct は Default + mutation で構築
        let mut batch_cfg = BatchConfig::default();
        batch_cfg.limits = RenderLimits::builder().max_document_pages(n).build();
        batch_cfg.initial_registry = Some(plan.target_registry);
        raikiri::render_batch(&doc, defaults, resolver.as_ref(), batch_cfg, &mut sink)?
    }
    _ => {
        // Streaming: 大規模、DoS 耐性最大
        let mut streaming_cfg = StreamingConfig::default();
        streaming_cfg.initial_registry = Some(plan.target_registry);
        raikiri::render_streaming(&doc, defaults, resolver.as_ref(), streaming_cfg, &mut sink)?
    }
};

match status {
    RenderStatus::Completed(summary) => {
        for warning in &summary.warnings {
            log::warn!("{:?}", warning);
        }
    }
    RenderStatus::Aborted { partial_pages } => {
        log::info!("Aborted after {} pages", partial_pages);
    }
    _ => {  // round 4 review #5: #[non_exhaustive] enum の forward-compat
        log::warn!("Unknown RenderStatus variant");
    }
}

// 4. (optional) Consumer 主導の iteration
//    target_discrepancies が空でなければ、Consumer 判断で再度 plan+render
//    fulgur は untrusted input なら iter=0、trusted なら iter=N 等を config で強制

// 5. FulgurPdfSink が PageFragment を walk して krilla に落とす
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

- 縦書き / ルビ / JIS X 4051 相当の日本語組版拡張タイミング
- **M0-gated decisions** (round 4 review #6 対応、M0 outcome 確定まで retention):
  - parley `FontContext` の `Sync` 適合性 (rayon 並列 shape の前提)
  - taffy の並列 layout 対応可否 (multi-column の各 column 並列)
  - selectors + cssparser の version 整合
  - anyrender::PaintScene の adapter surface 確定
  - anyrender_vello_cpu の byte-identical 保証
  M0 feasibility report で確定次第、Open Questions から Design 決定に昇格する
  (`OK` なら仕様維持、`NEEDS_DESIGN_CHANGE` なら architecture 変更 + doc revise)
- `SandboxedNetProvider` の spec 詳細 (URL allowlist、size cap、MIME
  whitelist の具体的な interface)
- Consumer iteration の推奨実装 example の充実 (fulgur の tier 別 policy に応じた
  `max_target_iterations` 設定ガイド)
- `plan` に raster 見積り情報 (`estimated_raster_bytes`, pixel dim per
  DPI 等) を追加する検討 — Consumer の memory 予測に有用 (優先度低)
- `raikiri-blitz-compat` の sunset タイミング (fulgur 完全 migration 後)
- 将来的な `raikiri-paint-pdf` (anyrender::PaintScene 実装、krilla base) の
  raikiri 側追加 vs. fulgur 側維持
- WASM 対応 (`raikiri-vrt` は wasm でも動くべきか)

## 16. 用語集

- **Consumer**: raikiri を使う側 (fulgur、将来の EPUB reader、他)
- **RenderSink**: Consumer が実装する trait、`accept_page(page)` で per-page
  受取り、`finish_render(summary)` で最終 TargetRegistry を受取り (Finding #4)
- **RenderSummary**: `finish_render` の引数、target_registry / unresolved_targets
  / emitted_target_slots / **target_discrepancies** (Finding #5 対応) を含む
- **DocumentPlan** (Finding #5 対応): `plan` の返り値、
  total_pages / target_registry (hint) / target_definitions /
  unresolved_targets / page_summary を含む。PaintedBox tree は含まない
- **PageSummary**: DocumentPlan 内、per-page の page_box / break_reason /
  target_slot_count / target_definition_count / content_height
- **TargetDiscrepancy**: RenderSummary 内、hint 値と実測値の乖離
  (fragment_id / hinted_page / actual_page / hinted_text / actual_text)
- **`plan`**: dry-run entry point (`plan(&Document, ...)`)。PaintScene / PaintedBox 構築なし、
  cost 見積り + target hint 生成 + DoS 防御ゲート用
- **StreamingConfig / BatchConfig**: render_* の config。lookahead と
  initial_registry (hint) を含む
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
  (1 pass、UnboundedLookahead + Fragmentation L3 対応)
- **DOM cursor / Emission cursor** (Finding #2 対応): 2 cursor モデル。DOM
  cursor は自由に peek ahead、Emission cursor は PageFragment emit 時のみ進む。
  ギャップ = look-ahead 幅
- **PageBoxCache**: 同じ (page_name, parity, is_first, is_blank) の @page 解決
  結果をキャッシュ、layout hot path の最適化
- **Security 2 層 model** (Finding #6 対応): Layer 1 (DOM sanitize via TreeSink
  wrap) + Layer 2 (Resource policy via SandboxedNetProvider / SandboxedResolver)
- **ResourcePolicy** (trait): scheme / host / size / MIME / timeout / redirect /
  recursion 制御を集約
- **SandboxedNetProvider / SandboxedResolver** (raikiri-net): NetworkProvider /
  ReplacedResolver を wrap して policy を適用する decorator
- **DefaultSandboxPolicy** (raikiri-net): allowed_schemes=[https,data]、10 MB
  上限、5s timeout 等の実用的 default preset
- **DenyAllPolicy** (raikiri-net): 全 fetch を reject する最も restrictive な
  出発点 preset
- **ResourceKind**: fetch context (StylesheetImport / ExternalStylesheet /
  Image / Font / Svg / MathML / Other) — policy method に context を渡す
- **RenderError** (Finding #10 対応、round 3 訂正): 構造化 error enum、全 variant が
  terminal。Parse / Cascade / Layout / Resolver / Network / Policy /
  PageLimitExceeded / Sink / Configuration / Io。`#[non_exhaustive]`。
  以前 `is_recoverable()` / `is_fatal()` 分類は round 3 で撤回、fallback は
  Consumer 側 impl 内で `Ok(fallback)` 返却で表現
- **RenderStatus** (round 3 review #1 対応): render_* の Ok 側戻り値。
  `Completed(RenderSummary)` = 全ページ emit + finish_render 成功、
  `Aborted { partial_pages }` = AbortSignal による graceful shutdown
- **GcpmSnapshot** (Missing consideration 対応): 並列 margin box layout 用の
  owned immutable snapshot。counter tree / string 4-snapshot / running
  bindings / target snapshot の deep copy
- **MarginBoxCounterScratch** (round 3 Missing #5 対応): 各 margin box worker
  が独自に持つ counter mutation buffer、layout 完了時に discard
- **AbortSignal / AbortController**: blitz-traits 準拠、Consumer が abort を
  発火し raikiri は cooperative check points で検知
- **Reference documents** (Finding #9 対応、T1): `tests/reference/` 配下の
  fulgur ユースケース由来の代表ドキュメント (invoice、report、contract、
  certificate 等)。ship 判定基準
- **T1 / T2 / T3 tier** (Finding #9 対応):
  - T1: reference documents (ship 判定)
  - T2: regression detection (品質維持、scope 内外 flat rule)
  - T3: WPT / blitz oracle の trend 追跡 (非 blocking)
- **DynamicFlags** (Finding #8 対応): running template の "何が dynamic か"
  記録するフラグ (has_counter / has_string / has_target / has_content_variant)
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
