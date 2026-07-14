# raikiri-traits populate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `crates/raikiri-traits/` に、設計仕様書 §4 の raikiri-traits 記述を Rust コードとして一括 populate する (error taxonomy 型を含む)。

**Architecture:** Theme 別に 10 個の submodule (`sink.rs`, `resolver.rs`, `net.rs`, `policy.rs`, `error.rs`, `strategy.rs`, `dom.rs`, `page.rs`, `config.rs`, `plan.rs`) に分割。`lib.rs` は `pub mod` と主要型の `pub use` のみ。Category A 型 (§4 で field 全記載) は完全定義、Category B 型 (trait 定義) は method signature 完全記述、Category C 型 (§5/§7/§9/§11 参照 opaque 型) は non_exhaustive struct + impl Default + pub fn new() の placeholder として置く。

**Tech Stack:** Rust 2024 edition, MSRV 1.89, workspace deps (smol_str, bytes, url, rustc-hash), std only (no additional external dep)。

## Global Constraints

- **Authoritative spec section**: `docs/superpowers/specs/2026-07-13-raikiri-rebuild-design.md` §4 が型定義の唯一の source of truth (§13.0 spec drift protocol 準拠)。narrative §5-§12 は補助資料。
- **Feasibility material**: `docs/feasibility-report.md` §3 (M0 spike から確定した型 pattern)。
- **beads issue**: `raikiri-spike-m1.1`。design + acceptance フィールド保存済み。`bd show raikiri-spike-m1.1` で全文取得。
- **workspace lints**: `unsafe_code = "deny"`, `missing_docs = "warn"`。全 pub item に doc comment 必須。placeholder 型は局所的に `#[allow(missing_docs)]`。
- **`#[non_exhaustive]` policy**: §4 で明示された全 pub struct / enum に付与。`impl Default` + `pub fn new() -> Self { Self::default() }` を強制。
- **Finding number trace**: §4 doc の `Finding #N` / `round X review #Y` 記述を doc comment に転記 (spec との trace)。
- **Branch**: `raikiri-spike-m1.1` (worktree `.claude/worktrees/raikiri-spike-m1.1/`)。
- **Verification per task**: 各 task 末尾で `cargo build -p raikiri-traits` と `cargo test -p raikiri-traits` が green。
- **Final verification (Task 12)**: `cargo build -p raikiri-traits`, `cargo test -p raikiri-traits`, `cargo clippy -p raikiri-traits -- -D warnings` が全 pass。

---

## File Structure

- Create: `crates/raikiri-traits/src/dom.rs` — Symbol / NodeId newtype + Dom / Element / Node shell traits
- Create: `crates/raikiri-traits/src/page.rs` — PageFragment / PageBox / PageContext / LayoutBuffer / TargetRegistry / GcpmDirective / ContentValueItem / RunningTemplate (全て opaque)
- Create: `crates/raikiri-traits/src/policy.rs` — ResourcePolicy trait + ResourceKind + PolicyViolation + ViolationType
- Create: `crates/raikiri-traits/src/net.rs` — NetworkProvider + Request / FetchedResource / Body / Method / AbortSignal / AbortController / NetworkError / HeaderMap / FormData
- Create: `crates/raikiri-traits/src/resolver.rs` — ReplacedResolver + ResolvedIntrinsic / ResolveDisposition / IntrinsicBox / ResolverError / ResolverRequest
- Create: `crates/raikiri-traits/src/error.rs` — RenderError / RenderStatus / RenderSummary / RenderWarning / WarningKind / ExhaustionPolicy / LimitKind / ParseError / CascadeError / LayoutError / UnresolvedTarget / UnresolvedReason / EmittedSlotInfo / TargetSlotId / TargetKind
- Create: `crates/raikiri-traits/src/sink.rs` — RenderSink trait
- Create: `crates/raikiri-traits/src/strategy.rs` — LookaheadPolicy / TargetResolver / EmissionPolicy / ReflowPolicy + supporting types
- Create: `crates/raikiri-traits/src/config.rs` — RenderLimits / LookaheadConfig / StreamingConfig / BatchConfig / PlanConfig + 5 Builder
- Create: `crates/raikiri-traits/src/plan.rs` — DocumentPlan / PageSummary / TargetDefinition / TargetDiscrepancy / BreakReason
- Modify: `crates/raikiri-traits/src/lib.rs` — 全 mod 宣言 + 主要型 re-export + inline unit tests

**Dependency order (Task 番号順に build)**:
1. dom.rs (standalone)
2. page.rs (opaque placeholders, standalone)
3. policy.rs (uses url::Url only)
4. net.rs (uses policy::PolicyViolation)
5. resolver.rs (uses url::Url only)
6. error.rs (uses net, policy, resolver, page, dom)
7. sink.rs (uses error::RenderSummary, page::PageFragment)
8. strategy.rs (uses sink, page, error)
9. config.rs (uses page::TargetRegistry)
10. plan.rs (uses page, error, dom)
11. lib.rs re-exports + tests
12. Acceptance verification + close

---

## Task 1: dom.rs — Foundation types (Symbol / NodeId / Dom / Element / Node shell)

**Files:**
- Create: `crates/raikiri-traits/src/dom.rs`
- Modify: `crates/raikiri-traits/src/lib.rs`

**Interfaces:**
- Consumes: `smol_str::SmolStr`
- Produces:
  - `pub struct Symbol(pub SmolStr)` — 一意識別子 (target id / fragment id 用)
  - `pub struct NodeId(pub u64)` — DOM node の一意識別子
  - `pub trait Dom` — shell (method / associated type 未確定、M1.5 で拡充)
  - `pub trait Element<'a>` — shell
  - `pub trait Node<'a>` — shell

- [ ] **Step 1: Create `crates/raikiri-traits/src/dom.rs`**

```rust
//! DOM abstraction trait + identifier newtypes.
//!
//! `Dom` / `Element<'a>` / `Node<'a>` は M1.5 (`dom-model` task) で
//! associated type + method を確定する予定。M1.1 では shell として trait だけ
//! 用意し、raikiri-dom 側の実装検討と co-design する。

use smol_str::SmolStr;

/// String identifier for GCPM fragments / running templates / named strings /
/// target-* references.
///
/// SmolStr newtype で inline 最適化を効かせる。M4 GCPM で本格利用開始。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Symbol(pub SmolStr);

impl Symbol {
    /// Construct from any string type.
    pub fn new(s: impl Into<SmolStr>) -> Self {
        Self(s.into())
    }

    /// Borrow inner str.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl From<&str> for Symbol {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

/// Stable identifier for DOM nodes across a document.
///
/// `RenderWarning.node_id` などで参照される。raikiri-dom は自 arena の
/// node index を u64 に射影して produce。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u64);

impl NodeId {
    /// Construct a node identifier from a raw u64.
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

/// DOM tree abstraction consumed by raikiri-dom / raikiri-style / raikiri-paint.
///
/// M1.1 では shell (method 未定義)。M1.5 `dom-model` で associated type と
/// query method を確定する予定。設計仕様書 §4 参照。
pub trait Dom {
    // M1.5 で populate:
    //   type ElementRef<'a>: Element<'a>;
    //   fn document_element(&self) -> Self::ElementRef<'_>;
    //   fn node(&self, id: NodeId) -> Option<Self::NodeRef<'_>>;
    //   ...
}

/// Element reference abstraction (M1.5 で拡充)。
pub trait Element<'a> {
    // M1.5 で populate:
    //   fn tag_name(&self) -> &str;
    //   fn attribute(&self, name: &str) -> Option<&str>;
    //   ...
}

/// Node reference abstraction (M1.5 で拡充)。
pub trait Node<'a> {
    // M1.5 で populate:
    //   fn node_type(&self) -> NodeKind;
    //   ...
}
```

- [ ] **Step 2: Update `crates/raikiri-traits/src/lib.rs` to declare and re-export**

置換対象 (M0 stub の全内容):
```rust
//! raikiri-traits — foundation traits and neutral model types.
//!
//! M0 stub. Full trait / type surface (spec §4 raikiri-traits) is populated
//! in M1 traits-definition + error-taxonomy-types tasks.
```

新しい内容:
```rust
//! raikiri-traits — foundation traits and neutral model types.
//!
//! 設計仕様書 §4 に定義された全 trait / 中立モデル型を集約する。実装は持たず、
//! raikiri-html / raikiri-style / raikiri-dom / raikiri-paint / raikiri-net が
//! 参照する共通型層。詳細は `docs/superpowers/specs/2026-07-13-raikiri-rebuild-design.md`
//! §4 を参照。

pub mod dom;

pub use dom::{Dom, Element, NodeId, Node, Symbol};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_construct_from_str() {
        let s = Symbol::from("target-1");
        assert_eq!(s.as_str(), "target-1");
    }

    #[test]
    fn nodeid_construct() {
        let n = NodeId::new(42);
        assert_eq!(n.0, 42);
    }
}
```

- [ ] **Step 3: Verify build + test pass**

Run: `cargo build -p raikiri-traits && cargo test -p raikiri-traits`
Expected: build green, 2 tests pass。

- [ ] **Step 4: Commit**

```bash
cd .claude/worktrees/raikiri-spike-m1.1
git add crates/raikiri-traits/src/dom.rs crates/raikiri-traits/src/lib.rs
git commit -m "feat(raikiri-traits): add dom module — Symbol / NodeId / shell traits (m1.1)"
```

---

## Task 2: page.rs — Opaque placeholder types

**Files:**
- Create: `crates/raikiri-traits/src/page.rs`
- Modify: `crates/raikiri-traits/src/lib.rs`

**Interfaces:**
- Consumes: なし (opaque types)
- Produces:
  - `PageFragment`, `PageBox`, `PageContext`, `LayoutBuffer`, `TargetRegistry`, `RunningTemplate`, `FormData` — non_exhaustive struct + Default + new()
  - `GcpmDirective`, `ContentValueItem` — non_exhaustive enum (variant なし、M4 で populate)

- [ ] **Step 1: Create `crates/raikiri-traits/src/page.rs`**

```rust
//! Page-related neutral model types.
//!
//! ここに集めた型は §5 (Pipeline), §7 (GCPM), §9 (PageBox), §11 (Paint) が
//! authoritative なので、M1.1 では opaque placeholder として置き、後続の
//! task が field / method を段階的に populate する。
//!
//! 全 struct は `#[non_exhaustive]` + `impl Default` + `pub fn new()` を持ち、
//! external consumer crate から `X::default()` で construct 可能。

// PageFragment — 1 ページの painted output (glyph run / decoration / target slot 含む)
// M1.7 paint-basic + M2 pagestream で populate。
#[allow(missing_docs)]
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct PageFragment {
    // M1.7 / M2 で populate:
    //   pub page_index: u32,
    //   pub page_box: PageBox,
    //   pub items: Vec<PaintedBoxItem>,
    //   pub target_slots: Vec<TargetSlot>,
    //   ...
}

impl PageFragment {
    /// Construct an empty PageFragment. M1.1 placeholder.
    pub fn new() -> Self { Self::default() }
}

// PageBox — @page rule 解決結果 (size, margins, margin box slots)
// M1.6 layout-single-page で populate。
#[allow(missing_docs)]
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct PageBox {
    // M1.6 で populate:
    //   pub width: f32,
    //   pub height: f32,
    //   pub margins: Margins,
    //   pub margin_boxes: [Option<MarginBox>; 16],
    //   ...
}

impl PageBox {
    /// Construct an empty PageBox. M1.1 placeholder.
    pub fn new() -> Self { Self::default() }
}

// PageContext — GCPM runtime state (counter tree, named string 4-snapshot,
// running bindings)
// M4 GCPM で populate。
#[allow(missing_docs)]
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct PageContext {
    // M4 で populate。
}

impl PageContext {
    /// Construct an empty PageContext. M1.1 placeholder.
    pub fn new() -> Self { Self::default() }
}

// LayoutBuffer — widow / orphan / break-inside / container probe lookahead
// buffer の中立モデル。実装は raikiri-dom 側 (§5 参照)。
// M2 layoutbuffer-skeleton で populate。
#[allow(missing_docs)]
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct LayoutBuffer {
    // M2 で populate。
}

impl LayoutBuffer {
    /// Construct an empty LayoutBuffer. M1.1 placeholder.
    pub fn new() -> Self { Self::default() }
}

// TargetRegistry — target-* placeholder emit + resolve の runtime registry。
// M4 target-* で populate (§7.2 参照)。
#[allow(missing_docs)]
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct TargetRegistry {
    // M4 で populate。
}

impl TargetRegistry {
    /// Construct an empty TargetRegistry. M1.1 placeholder.
    pub fn new() -> Self { Self::default() }
}

// RunningTemplate — `position: running(name)` の template 登録
// M4 で populate。
#[allow(missing_docs)]
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct RunningTemplate {
    // M4 で populate。
}

impl RunningTemplate {
    /// Construct an empty RunningTemplate. M1.1 placeholder.
    pub fn new() -> Self { Self::default() }
}

// FormData — application/x-www-form-urlencoded body の中立モデル
// Consumer 側 network 実装で参照 (Body::Form(FormData))。
#[allow(missing_docs)]
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct FormData {
    // 将来 populate:
    //   pub pairs: Vec<(String, String)>,
}

impl FormData {
    /// Construct an empty FormData. M1.1 placeholder.
    pub fn new() -> Self { Self::default() }
}

/// GCPM directive emitted by raikiri-style cascade (`counter-increment` /
/// `string-set` / `position: running(name)` 等)。
///
/// M4 で variant を populate (§7.1 参照)。M1.1 では uninhabited。
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum GcpmDirective {
    // M4 で populate:
    //   CounterIncrement { name: Symbol, delta: i32 },
    //   CounterReset { name: Symbol, value: i32 },
    //   StringSet { name: Symbol, value: Vec<ContentValueItem> },
    //   RunningRegister { name: Symbol },
}

/// resolved `content` property の item (`content: string(...)`, `counter(...)`,
/// `target-counter(...)`, `element(...)` 等の資産)。
///
/// M4 で variant を populate (§7 参照)。M1.1 では uninhabited。
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum ContentValueItem {
    // M4 で populate:
    //   Literal(String),
    //   Counter { name: Symbol, style: CounterStyle },
    //   String { name: Symbol, snapshot: NamedStringSnapshot },
    //   TargetCounter { url: Url, name: Symbol, style: CounterStyle },
    //   ...
}
```

- [ ] **Step 2: Update `crates/raikiri-traits/src/lib.rs`**

`pub mod dom;` の次行に追加:

```rust
pub mod page;
```

`pub use dom::{...};` の次行に追加:

```rust
pub use page::{
    ContentValueItem, FormData, GcpmDirective, LayoutBuffer, PageBox, PageContext,
    PageFragment, RunningTemplate, TargetRegistry,
};
```

- [ ] **Step 3: Add construction test to `lib.rs::tests`**

`lib.rs` の `#[cfg(test)] mod tests` に以下追加:

```rust
    #[test]
    fn page_placeholders_default_construct() {
        let _ = PageFragment::default();
        let _ = PageFragment::new();
        let _ = PageBox::default();
        let _ = PageContext::default();
        let _ = LayoutBuffer::default();
        let _ = TargetRegistry::default();
        let _ = RunningTemplate::default();
        let _ = FormData::default();
    }
```

- [ ] **Step 4: Verify build + test pass**

Run: `cargo build -p raikiri-traits && cargo test -p raikiri-traits`
Expected: build green, 3 tests pass。

- [ ] **Step 5: Commit**

```bash
git add crates/raikiri-traits/src/page.rs crates/raikiri-traits/src/lib.rs
git commit -m "feat(raikiri-traits): add page module — opaque placeholder types (m1.1)"
```

---

## Task 3: policy.rs — ResourcePolicy trait + supporting types

**Files:**
- Create: `crates/raikiri-traits/src/policy.rs`
- Modify: `crates/raikiri-traits/src/lib.rs`

**Interfaces:**
- Consumes: `url::Url`, `std::time::Duration`
- Produces:
  - `pub trait ResourcePolicy: Send + Sync` — 9 method
  - `pub enum ResourceKind` (#[non_exhaustive]) — 7 variant
  - `pub struct PolicyViolation` — 4 field
  - `pub enum ViolationType` — 8 variant

- [ ] **Step 1: Create `crates/raikiri-traits/src/policy.rs`**

```rust
//! Resource policy trait + policy violation types.
//!
//! `SandboxedNetProvider` / `SandboxedResolver` (raikiri-net) が消費する policy。
//! Finding #6 対応。

use std::time::Duration;
use url::Url;

/// Resource fetch / decode に対する policy 判定 trait。
///
/// raikiri-net の `SandboxedNetProvider<P>` / `SandboxedResolver<R>` が
/// 各 method を pre-fetch / post-fetch phase で呼び分ける。Consumer が
/// custom policy を実装するか、`raikiri-net::DefaultSandboxPolicy` を利用。
///
/// Finding #6 対応 (round 7 未対応 finding: redirect / timeout / recursion 系
/// method の削除は M4 sandboxed-net-provider-impl 前に確定)。
pub trait ResourcePolicy: Send + Sync {
    /// URL scheme (`https` / `data` / `file` / ...) が許可されているか。
    fn is_scheme_allowed(&self, scheme: &str, kind: ResourceKind) -> bool;

    /// host が許可されているか。
    fn is_host_allowed(&self, host: &str, kind: ResourceKind) -> bool;

    /// redirect を許可するか。
    fn allow_redirect(&self, from: &Url, to: &Url, hop: u32) -> bool;

    /// redirect の最大 hop 数。
    fn max_redirect_hops(&self, kind: ResourceKind) -> u32;

    /// fetch 前の最大 byte 数 (Content-Length ベース、DoS 対策)。
    fn max_fetch_bytes(&self, kind: ResourceKind) -> Option<u64>;

    /// decode 後の最大 byte 数 (展開後 memory footprint 対策)。
    fn max_decoded_bytes(&self, kind: ResourceKind) -> Option<u64>;

    /// fetch 全体の timeout (thread hang 対策)。
    fn fetch_timeout(&self, kind: ResourceKind) -> Duration;

    /// decode の timeout。
    fn decode_timeout(&self, kind: ResourceKind) -> Duration;

    /// 許可される MIME type list (`text/css`, `image/png`, ...)。
    fn allowed_mime_types(&self, kind: ResourceKind) -> Vec<String>;

    /// chained `@import` の最大 depth。
    fn max_import_depth(&self) -> u32;

    /// 外部 SVG recursion の最大 depth。
    fn max_svg_recursion_depth(&self) -> u32;
}

/// Fetch した resource の分類 (policy 判定の context)。
///
/// Finding #6 対応。§4 に列挙された 7 variant を再現。将来拡張のため
/// `#[non_exhaustive]`。
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    /// `@import` inside CSS。
    StylesheetImport,
    /// `<link rel="stylesheet">` から fetch する外部 stylesheet。
    ExternalStylesheet,
    /// `<img src>`, `background-image` 等。
    Image,
    /// `@font-face src`。
    Font,
    /// 外部 SVG。
    Svg,
    /// 外部 MathML。
    MathML,
    /// Fallback。
    Other,
}

/// Policy 違反の詳細情報。
#[derive(Debug, Clone)]
pub struct PolicyViolation {
    /// どの resource kind で発生した違反か。
    pub kind: ResourceKind,
    /// 対象 URL。
    pub url: Url,
    /// 違反 type。
    pub violation_type: ViolationType,
    /// 人間可読な詳細 message。
    pub details: String,
}

/// Policy 違反の分類。
///
/// §4 の 8 variant を再現。round 7 未対応 finding: redirect / timeout /
/// recursion 系は M4 で `ResourcePolicy` から削除される可能性あり。
#[derive(Debug, Clone)]
pub enum ViolationType {
    /// URL scheme が `is_scheme_allowed` で reject。
    SchemeNotAllowed,
    /// host が `is_host_allowed` で reject。
    HostNotAllowed,
    /// redirect が `allow_redirect` で reject。
    RedirectDenied,
    /// fetch 済 byte 数が `max_fetch_bytes` 超過。
    FetchTooLarge { limit: u64, actual: u64 },
    /// decode 済 byte 数が `max_decoded_bytes` 超過。
    DecodedTooLarge { limit: u64, actual: u64 },
    /// fetch / decode timeout 超過。
    Timeout,
    /// MIME type が `allowed_mime_types` に無い。
    MimeNotAllowed { mime: String },
    /// `@import` / SVG recursion depth 超過。
    RecursionExceeded { depth: u32 },
    /// その他。
    Other,
}
```

- [ ] **Step 2: Update `crates/raikiri-traits/src/lib.rs`**

`pub mod page;` の後に追加:

```rust
pub mod policy;
```

re-export に追加:

```rust
pub use policy::{PolicyViolation, ResourceKind, ResourcePolicy, ViolationType};
```

- [ ] **Step 3: Add object-safety test to `lib.rs::tests`**

```rust
    #[test]
    fn resource_policy_is_object_safe() {
        fn _assert<T: ?Sized>() {}
        _assert::<dyn ResourcePolicy>();
    }
```

- [ ] **Step 4: Verify build + test pass**

Run: `cargo build -p raikiri-traits && cargo test -p raikiri-traits`
Expected: build green, 4 tests pass。

- [ ] **Step 5: Commit**

```bash
git add crates/raikiri-traits/src/policy.rs crates/raikiri-traits/src/lib.rs
git commit -m "feat(raikiri-traits): add policy module — ResourcePolicy trait + kinds (m1.1)"
```

---

## Task 4: net.rs — NetworkProvider + Request/Response types

**Files:**
- Create: `crates/raikiri-traits/src/net.rs`
- Modify: `crates/raikiri-traits/src/lib.rs`

**Interfaces:**
- Consumes: `url::Url`, `bytes::Bytes`, `std::sync::{Arc, atomic::{AtomicBool, Ordering}}`, `policy::PolicyViolation`, `page::FormData`
- Produces:
  - `pub trait NetworkProvider: Send + Sync`
  - `pub type HeaderMap = Vec<(String, String)>`
  - `pub struct Request`, `pub struct FetchedResource`
  - `pub enum Body`, `pub enum Method` (#[non_exhaustive])
  - `pub struct AbortSignal`, `pub struct AbortController`
  - `pub enum NetworkError`

- [ ] **Step 1: Create `crates/raikiri-traits/src/net.rs`**

```rust
//! Network provider trait + neutral network types.
//!
//! blitz-traits::NetProvider の shape に揃える (Finding #6 対応)。raikiri は
//! sync core のため callback ではなく sync return。policy 適用は
//! `raikiri-net::SandboxedNetProvider` による wrap で行う。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use url::Url;

use crate::page::FormData;
use crate::policy::{PolicyViolation, ResourceKind};

/// HTTP header の list。同名 header の複数値も表現可能。
///
/// 軽量 shape に留めるため `http::HeaderMap` を採用せず `Vec<(String, String)>`。
/// M6 blitz-compat 実装時に必要なら再検討。
pub type HeaderMap = Vec<(String, String)>;

/// Consumer が実装する sync-return の network provider trait。
///
/// raikiri は timer thread を持たない (round 5 review #2)。timeout は
/// Consumer が自身の async runtime / thread pool で管理する。
///
/// Finding #6 対応 (round 7 未対応 finding: byte enforcement 戦略確定は
/// M4 sandboxed-net-provider-impl 前)。
pub trait NetworkProvider: Send + Sync {
    /// 1 fetch を同期実行し、結果か error を返す。
    fn fetch(&self, request: Request) -> Result<FetchedResource, NetworkError>;
}

/// Fetch 要求の全情報。
#[derive(Debug, Clone)]
pub struct Request {
    /// Target URL (redirect 前)。
    pub url: Url,
    /// HTTP method。
    pub method: Method,
    /// Content-Type header (POST body 用)。
    pub content_type: Option<String>,
    /// 追加 header (`Accept`, `Referer`, custom 等)。
    pub headers: HeaderMap,
    /// Request body。
    pub body: Body,
    /// AbortSignal (option、Consumer が渡す)。
    pub signal: Option<AbortSignal>,
    /// raikiri 追加: fetch の目的 (blitz は doc_id、raikiri は context 表現)。
    pub kind: ResourceKind,
}

/// Fetch 成功時の response。
#[derive(Debug, Clone)]
pub struct FetchedResource {
    /// Response body の生 bytes。
    pub bytes: Bytes,
    /// `Content-Type` header (parse 済み MIME type)。
    pub content_type: Option<String>,
    /// Redirect 後の実効 URL。
    pub final_url: Url,
    /// 明示された character encoding (無ければ MIME や BOM から推測)。
    pub encoding: Option<String>,
}

/// Request body 表現。
#[derive(Debug, Clone)]
pub enum Body {
    /// 生 bytes body。
    Bytes(Bytes),
    /// application/x-www-form-urlencoded body。
    Form(FormData),
    /// body 無し (GET 等)。
    Empty,
}

/// HTTP method (拡張余地あり、round 3 review #3 訂正: HTTP method は仕様上
/// 拡張可能なため `#[non_exhaustive]`)。
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Method {
    /// GET。
    Get,
    /// POST。
    Post,
    /// その他 (PUT / DELETE / PATCH / OPTIONS / HEAD ...)。
    Other(String),
}

/// blitz と同じ AbortSignal shape (AtomicBool ラッパ)。
///
/// Consumer が `AbortController::abort()` を呼ぶと、共有 AtomicBool が
/// `true` になり、raikiri と Consumer の両側から観測可能。
#[derive(Debug, Clone)]
pub struct AbortSignal(Arc<AtomicBool>);

impl AbortSignal {
    /// signal が abort されたか。
    pub fn is_aborted(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// AbortSignal を produce する controller。round 5 review #2 対応: raikiri は
/// timer thread を一切 spawn しない。timeout は Consumer が自分の async
/// runtime で管理する。
#[derive(Debug, Default)]
pub struct AbortController {
    /// このコントローラが管理する signal。
    pub signal: AbortSignal,
}

impl AbortController {
    /// 新規 controller を生成。
    pub fn new() -> Self {
        Self { signal: AbortSignal(Arc::new(AtomicBool::new(false))) }
    }

    /// signal を abort 状態に遷移させる。
    pub fn abort(&self) {
        self.signal.0.store(true, Ordering::Release);
    }
}

impl Default for AbortSignal {
    fn default() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
}

/// Network 層 error。§4 の 5 variant を再現。
#[derive(Debug)]
pub enum NetworkError {
    /// AbortSignal によって中断された。
    Aborted,
    /// ResourcePolicy 違反 (SandboxedNetProvider が発火)。
    PolicyViolation(PolicyViolation),
    /// I/O error。
    Io(std::io::Error),
    /// HTTP status code error (4xx / 5xx)。
    Http(u16),
    /// その他。
    Other(String),
}
```

- [ ] **Step 2: Update `crates/raikiri-traits/src/lib.rs`**

```rust
pub mod net;
```

```rust
pub use net::{
    AbortController, AbortSignal, Body, FetchedResource, HeaderMap, Method,
    NetworkError, NetworkProvider, Request,
};
```

- [ ] **Step 3: Add tests to `lib.rs::tests`**

```rust
    #[test]
    fn network_provider_is_object_safe() {
        fn _assert<T: ?Sized>() {}
        _assert::<dyn NetworkProvider>();
    }

    #[test]
    fn abort_controller_default_and_abort() {
        let c = AbortController::new();
        assert!(!c.signal.is_aborted());
        c.abort();
        assert!(c.signal.is_aborted());
    }
```

- [ ] **Step 4: Verify build + test pass**

Run: `cargo build -p raikiri-traits && cargo test -p raikiri-traits`
Expected: build green, 6 tests pass。

- [ ] **Step 5: Commit**

```bash
git add crates/raikiri-traits/src/net.rs crates/raikiri-traits/src/lib.rs
git commit -m "feat(raikiri-traits): add net module — NetworkProvider + Request/Response types (m1.1)"
```

---

## Task 5: resolver.rs — ReplacedResolver + resolve types

**Files:**
- Create: `crates/raikiri-traits/src/resolver.rs`
- Modify: `crates/raikiri-traits/src/lib.rs`

**Interfaces:**
- Consumes: `url::Url`
- Produces:
  - `pub trait ReplacedResolver`
  - `pub struct IntrinsicBox` (opaque placeholder)
  - `pub struct ResolvedIntrinsic`, `pub enum ResolveDisposition` (#[non_exhaustive])
  - `pub struct ResolverRequest<'a>` (placeholder), `pub enum ResolverError` (#[non_exhaustive], variant 空)

- [ ] **Step 1: Create `crates/raikiri-traits/src/resolver.rs`**

```rust
//! Replaced-element resolver trait + resolve types.
//!
//! Consumer が `<img>`, `<object>`, `<embed>`, `<svg>` 等の replaced element の
//! intrinsic size を返す trait。size 決定 だけを扱う (fetch は Consumer 側)。

use std::marker::PhantomData;

use url::Url;

/// Replaced element の intrinsic size を resolve する Consumer 側 trait。
///
/// - req に含まれる URL の scheme / host / size 等の検証は Consumer 責任。
///   集中的に policy を効かせたい場合は `raikiri-net::SandboxedResolver` で wrap
///   (Finding #6 対応、§10 参照)。
/// - round 4 review #3 対応: `Err` は常に terminal (`RenderError::Resolver`)。
///   Consumer 側 fallback は `Ok(ResolvedIntrinsic { intrinsic, disposition:
///   Fallback { .. } })` として返し、raikiri は disposition を見て
///   `RenderSummary.warnings` に自動記録する。
pub trait ReplacedResolver {
    /// 1 replaced element の resolve。
    fn resolve(&self, req: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError>;
}

/// Intrinsic size + metadata。M4 で fields を populate。M1.1 では opaque。
#[allow(missing_docs)]
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct IntrinsicBox {
    // M4 で populate:
    //   pub width: Option<f32>,
    //   pub height: Option<f32>,
    //   pub aspect_ratio: Option<f32>,
    //   pub baseline: Option<f32>,
    //   pub encoding: MediaEncoding,
    //   ...
}

impl IntrinsicBox {
    /// M1.1 placeholder constructor.
    pub fn new() -> Self { Self::default() }
}

/// Resolver からの正常 return 型。
#[derive(Debug, Clone)]
pub struct ResolvedIntrinsic {
    /// Intrinsic size + metadata。
    pub intrinsic: IntrinsicBox,
    /// Resolve disposition (`Ok` / `Fallback`)。
    pub disposition: ResolveDisposition,
}

/// Resolve 成功 / fallback の分類。
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum ResolveDisposition {
    /// 通常の resolve 成功。
    Ok,
    /// Consumer が意図的に fallback を選択 (image not found → placeholder 等)。
    /// raikiri は `WarningKind::ResolverFallback` として
    /// `RenderSummary.warnings` に記録する。
    Fallback {
        /// 人間可読な fallback 理由。
        reason: String,
    },
}

/// Resolve 対象 element の詳細 (borrowed reference)。
///
/// M4 で fields (element_kind / url / hint_size / attributes 等) を populate。
/// M1.1 では phantom lifetime marker のみ。
#[allow(missing_docs)]
#[derive(Debug)]
#[non_exhaustive]
pub struct ResolverRequest<'a> {
    // M4 で populate:
    //   pub url: &'a Url,
    //   pub element_kind: ReplacedElementKind,
    //   pub hint_size: Option<Size>,
    //   pub attributes: &'a Attributes,
    _marker: PhantomData<&'a ()>,
}

impl<'a> ResolverRequest<'a> {
    /// M1.1 placeholder constructor.
    pub fn new() -> Self {
        Self { _marker: PhantomData }
    }
}

impl<'a> Default for ResolverRequest<'a> {
    fn default() -> Self {
        Self::new()
    }
}

/// Resolver 層 error。M4 で variant を populate。M1.1 では uninhabited。
///
/// M4 想定 variant:
///   - `Io(std::io::Error)`
///   - `Decode(String)`
///   - `Timeout`
///   - `NotSupported { kind: ReplacedElementKind }`
#[non_exhaustive]
#[derive(Debug)]
pub enum ResolverError {
    // M4 で populate。
}

// URL は将来使用予定 (ResolverRequest.url field を M4 で追加時)。
// M1.1 では unused warning を避けるため referenced 表示。
#[allow(dead_code)]
fn _url_phantom(_u: &Url) {}
```

- [ ] **Step 2: Update `crates/raikiri-traits/src/lib.rs`**

```rust
pub mod resolver;
```

```rust
pub use resolver::{
    IntrinsicBox, ReplacedResolver, ResolveDisposition, ResolvedIntrinsic, ResolverError,
    ResolverRequest,
};
```

- [ ] **Step 3: Add tests to `lib.rs::tests`**

```rust
    #[test]
    fn replaced_resolver_is_object_safe() {
        fn _assert<T: ?Sized>() {}
        _assert::<dyn ReplacedResolver>();
    }

    #[test]
    fn resolver_placeholder_types_default_construct() {
        let _ = IntrinsicBox::default();
        let _ = ResolverRequest::default();
    }
```

- [ ] **Step 4: Verify build + test pass**

Run: `cargo build -p raikiri-traits && cargo test -p raikiri-traits`
Expected: build green, 8 tests pass。

- [ ] **Step 5: Commit**

```bash
git add crates/raikiri-traits/src/resolver.rs crates/raikiri-traits/src/lib.rs
git commit -m "feat(raikiri-traits): add resolver module — ReplacedResolver + intrinsic types (m1.1)"
```

---

## Task 6: error.rs — RenderError / RenderStatus + error taxonomy

**Files:**
- Create: `crates/raikiri-traits/src/error.rs`
- Modify: `crates/raikiri-traits/src/lib.rs`

**Interfaces:**
- Consumes: `url::Url`, `net::NetworkError`, `policy::PolicyViolation`, `resolver::ResolverError`, `page::TargetRegistry`, `dom::{NodeId, Symbol}`
- Produces:
  - `pub enum RenderError` (#[non_exhaustive], 11 variant), `pub enum RenderStatus`
  - `pub struct RenderSummary`, `pub struct RenderWarning`
  - `pub enum WarningKind`, `pub enum ExhaustionPolicy` (#[non_exhaustive]), `pub enum LimitKind` (#[non_exhaustive])
  - `pub enum ParseError`, `pub enum CascadeError`, `pub enum LayoutError` (すべて #[non_exhaustive], variant 空 placeholder)
  - `pub struct UnresolvedTarget`, `pub enum UnresolvedReason`
  - `pub struct EmittedSlotInfo`, `pub struct TargetSlotId`
  - `pub enum TargetKind` (#[non_exhaustive], placeholder)

- [ ] **Step 1: Create `crates/raikiri-traits/src/error.rs`**

```rust
//! Render error taxonomy + status + summary types.
//!
//! `RenderError` は terminal error (rendering が停止した場所を表す)。Consumer
//! 側 fallback は `Ok(fallback)` を返すことで表現し、raikiri は
//! `RenderSummary.warnings` に記録する (Finding #1 新 review 対応)。

use url::Url;

use crate::dom::{NodeId, Symbol};
use crate::net::NetworkError;
use crate::page::TargetRegistry;
use crate::policy::PolicyViolation;
use crate::resolver::ResolverError;

/// Terminal render error。すべての variant は "rendering がそこで停止した" を意味。
///
/// Finding #10 対応 (構造化 error taxonomy)。round 4 review #1 対応で
/// `LimitExceeded` に統一。
#[non_exhaustive]
#[derive(Debug)]
pub enum RenderError {
    /// HTML parse エラー。
    Parse(ParseError),
    /// CSS parse / cascade エラー。
    Cascade(CascadeError),
    /// Layout エラー。
    Layout(LayoutError),
    /// Consumer の resolver が Err を返した。
    Resolver(ResolverError),
    /// Consumer の network が Err を返した。
    Network(NetworkError),
    /// Resource policy 違反。
    Policy(PolicyViolation),
    /// `RenderLimits` の各種 limit 超過 (fail-fast、round 4 review #1 対応で
    /// 旧 `PageLimitExceeded` を `kind: Pages` で吸収)。
    LimitExceeded {
        /// どの limit を超過したか。
        kind: LimitKind,
        /// 設定された limit 値。
        limit: u64,
        /// 観測された実 value。
        actual: u64,
    },
    /// Consumer の sink method (accept_page / finish_render) が Err を返した。
    Sink(std::io::Error),
    /// Config 不整合 (BatchConfig.initial_registry が不正 等)。
    Configuration(String),
    /// target-* が `max_target_iterations` 内に収束しなかった (round 6 review #5
    /// 対応、Consumer が `ExhaustionPolicy::Error` を選択した場合のみ発生)。
    TargetDidNotConverge {
        /// 実行された iteration 数。
        iterations: u32,
    },
    /// その他 `std::io::Error` 系。
    Io(std::io::Error),
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(_) => write!(f, "HTML parse error"),
            Self::Cascade(_) => write!(f, "CSS cascade error"),
            Self::Layout(_) => write!(f, "Layout error"),
            Self::Resolver(_) => write!(f, "Replaced-element resolver error"),
            Self::Network(_) => write!(f, "Network provider error"),
            Self::Policy(v) => write!(f, "Resource policy violation: {:?}", v.violation_type),
            Self::LimitExceeded { kind, limit, actual } => {
                write!(f, "Render limit exceeded: {kind:?} (limit={limit}, actual={actual})")
            }
            Self::Sink(_) => write!(f, "Sink returned I/O error"),
            Self::Configuration(msg) => write!(f, "Configuration error: {msg}"),
            Self::TargetDidNotConverge { iterations } => {
                write!(f, "target-* did not converge in {iterations} iterations")
            }
            Self::Io(_) => write!(f, "I/O error"),
        }
    }
}

impl std::error::Error for RenderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sink(e) | Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

/// Limit exceeded の分類 (round 4 review #1 対応)。
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitKind {
    /// `max_document_pages` 超過。
    Pages,
    /// `max_dom_nodes` 超過。
    DomNodes,
    /// `max_target_slots` 超過。
    TargetSlots,
    /// `max_layout_buffer_entries` 超過。
    LayoutBufferEntries,
    /// `max_aggregate_bytes` 超過。
    AggregateBytes,
}

/// AbortSignal による graceful shutdown を error と別カテゴリで表現。
/// `render_*` は `Result<RenderStatus, RenderError>` を返す。
#[derive(Debug)]
pub enum RenderStatus {
    /// 全ページ emit 完了、`finish_render` も成功。
    Completed(RenderSummary),
    /// AbortSignal による中断。直前まで emit 済み、`finish_render` は呼ばれない。
    Aborted {
        /// 中断前に commit されたページ数。
        partial_pages: u32,
    },
}

/// Render 完了 summary (Finding #4 completion protocol)。
#[derive(Debug)]
pub struct RenderSummary {
    /// 総ページ数。
    pub total_pages: u32,
    /// target-* の最終 registry (Consumer が patch table の base に利用)。
    pub target_registry: TargetRegistry,
    /// 未解決 target list。
    pub unresolved_targets: Vec<UnresolvedTarget>,
    /// emit 済み target slot list。
    pub emitted_target_slots: Vec<EmittedSlotInfo>,
    /// hint と actual の乖離を検知した項目 (Finding #5 対応、Consumer 収束判定用)。
    pub target_discrepancies: Vec<TargetDiscrepancy>,
    /// Consumer's fallback usage / policy violation 等の警告 (Finding #1 新 review 対応)。
    pub warnings: Vec<RenderWarning>,
}

/// Render 警告 (fallback usage / policy warning / unresolved target 等)。
#[derive(Debug)]
pub struct RenderWarning {
    /// 警告 kind。
    pub kind: WarningKind,
    /// 関連 DOM node (option、element-level warning に付く)。
    pub node_id: Option<NodeId>,
    /// 人間可読な詳細。
    pub details: String,
}

/// 警告 kind。§4 の 5 variant を再現。
#[derive(Debug)]
pub enum WarningKind {
    /// Consumer の resolver が fallback を返した (`Ok(fallback_intrinsic)`)。
    ResolverFallback {
        /// 対象 fragment id。
        fragment_id: Symbol,
    },
    /// Consumer の network が fallback を返した。
    NetworkFallback {
        /// 対象 URL。
        url: Url,
    },
    /// Policy violation を Consumer の on_violation が Warn 扱いにした。
    PolicyWarning {
        /// 発火した違反。
        violation: PolicyViolation,
    },
    /// target-* 参照先が見つからず fallback_text で描画された。
    UnresolvedTarget {
        /// 対象 fragment id。
        fragment_id: Symbol,
    },
    /// target-* が `max_target_iterations` 内に収束しなかったが、Consumer が
    /// `ExhaustionPolicy::BestEffort` を選択したため best-effort render された
    /// (round 6 review #5 対応)。
    TargetConvergenceExhausted {
        /// 尽くした iteration 数。
        iterations: u32,
    },
}

/// Consumer の convergence loop が `max_target_iterations` を尽くしたときの挙動
/// (round 6 review #5 対応、silent 続行を禁じる)。
///
/// Consumer 側 iteration に関する契約なので、raikiri の `plan()` / `render_*`
/// API 内では消費されない (Consumer が自身の loop で参照する)。
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExhaustionPolicy {
    /// 未収束を error として上流に返す (保守的 default)。
    Error,
    /// 最後の registry で render、`WarningKind::TargetConvergenceExhausted` を
    /// 必ず `summary.warnings` に記録。
    BestEffort,
}

impl Default for ExhaustionPolicy {
    fn default() -> Self {
        Self::Error
    }
}

/// 未解決 target の詳細 (Finding #4 completion protocol)。
#[derive(Debug, Clone)]
pub struct UnresolvedTarget {
    /// slot 一意識別子。
    pub slot_id: TargetSlotId,
    /// 未解決の fragment id。
    pub fragment_id: Symbol,
    /// 未解決の理由。
    pub reason: UnresolvedReason,
}

/// UnresolvedTarget の理由 (Finding #4)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnresolvedReason {
    /// fragment id がどこにも定義されていない。
    NotFound,
    /// Consumer 側 policy でエラー扱い。
    ConsumerRejected,
    // ConvergenceFailed は削除 (raikiri 内 iteration しないため、Finding #5)
}

/// emit 済み target slot の詳細。
#[derive(Debug, Clone)]
pub struct EmittedSlotInfo {
    /// slot 一意識別子。
    pub slot_id: TargetSlotId,
    /// 対象 fragment id。
    pub fragment_id: Symbol,
    /// target 種別。
    pub kind: TargetKind,
}

/// slot の一意識別子 (Consumer が patch table の key に使う)。
///
/// (page_index, sequence) は decode 順で unique、byte-identical 保証あり
/// (Finding #4 completion protocol)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TargetSlotId {
    /// このページの 0-indexed page number。
    pub page_index: u32,
    /// ページ内での通し番号 (target-* 出現順、0-indexed)。
    pub sequence: u32,
}

/// target-* の種別 (target-counter / target-text / target-string 等)。
///
/// M4 target-* で variant を populate。M1.1 では uninhabited。
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    // M4 で populate:
    //   Counter,
    //   Text,
    //   String,
    //   Element,
}

/// hint と actual の乖離を検知した項目 (Consumer 収束判定用、Finding #5)。
#[derive(Debug, Clone)]
pub struct TargetDiscrepancy {
    /// 対象 fragment id。
    pub fragment_id: Symbol,
    /// hint 段階で予告された page (無い場合は None)。
    pub hinted_page: Option<u32>,
    /// 実 render での page。
    pub actual_page: u32,
    /// hint 段階のテキスト content (無い場合は None)。
    pub hinted_text: Option<String>,
    /// 実 render のテキスト content。
    pub actual_text: String,
}

/// HTML parse 段階の error。M1.2 error propagation task で variant populate。
///
/// M1.1 では uninhabited。html5ever error / doctype mismatch / io error 等を
/// M1.2 で追加。
#[non_exhaustive]
#[derive(Debug)]
pub enum ParseError {
    // M1.2 で populate:
    //   Io(std::io::Error),
    //   Html5ever(String),
    //   ...
}

/// CSS parse / cascade 段階の error。M1.4 css-cascade-basic task で variant populate。
///
/// M1.1 では uninhabited。
#[non_exhaustive]
#[derive(Debug)]
pub enum CascadeError {
    // M1.4 で populate:
    //   Parse(String),
    //   InvalidValue(String),
    //   ...
}

/// Layout 段階の error。M1.6 layout-single-page task で variant populate。
///
/// M1.1 では uninhabited。
#[non_exhaustive]
#[derive(Debug)]
pub enum LayoutError {
    // M1.6 で populate:
    //   TaffyError(String),
    //   ...
}
```

- [ ] **Step 2: Update `crates/raikiri-traits/src/lib.rs`**

```rust
pub mod error;
```

```rust
pub use error::{
    CascadeError, EmittedSlotInfo, ExhaustionPolicy, LayoutError, LimitKind, ParseError,
    RenderError, RenderStatus, RenderSummary, RenderWarning, TargetDiscrepancy, TargetKind,
    TargetSlotId, UnresolvedReason, UnresolvedTarget, WarningKind,
};
```

- [ ] **Step 3: Add tests to `lib.rs::tests`**

```rust
    #[test]
    fn render_error_is_error_trait() {
        fn _assert<T: std::error::Error>() {}
        _assert::<RenderError>();
    }

    #[test]
    fn exhaustion_policy_default_is_error() {
        assert_eq!(ExhaustionPolicy::default(), ExhaustionPolicy::Error);
    }

    #[test]
    fn target_slot_id_construct() {
        let id = TargetSlotId { page_index: 3, sequence: 7 };
        assert_eq!(id.page_index, 3);
        assert_eq!(id.sequence, 7);
    }
```

- [ ] **Step 4: Verify build + test pass**

Run: `cargo build -p raikiri-traits && cargo test -p raikiri-traits`
Expected: build green, 11 tests pass。

- [ ] **Step 5: Commit**

```bash
git add crates/raikiri-traits/src/error.rs crates/raikiri-traits/src/lib.rs
git commit -m "feat(raikiri-traits): add error module — RenderError taxonomy + summary types (m1.1)"
```

---

## Task 7: sink.rs — RenderSink trait

**Files:**
- Create: `crates/raikiri-traits/src/sink.rs`
- Modify: `crates/raikiri-traits/src/lib.rs`

**Interfaces:**
- Consumes: `error::RenderSummary`, `page::PageFragment`
- Produces: `pub trait RenderSink: Send`

- [ ] **Step 1: Create `crates/raikiri-traits/src/sink.rs`**

```rust
//! RenderSink trait — Consumer 側の page emission receiver。

use crate::error::RenderSummary;
use crate::page::PageFragment;

/// Consumer 側 render output receiver (Finding #4 completion protocol)。
///
/// 1 ページ確定ごとに `accept_page` が呼ばれる:
/// - Streaming preset: 逐次 (`ImmediateEmission`)
/// - Batch preset: 全 layout 完了後まとめて (`DeferredEmission`)
///
/// 全 `accept_page` 呼び出し完了後、`finish_render` が最終通知として呼ばれる。
///
/// Consumer 側 resource 解放 (PDF trailer 書出 等) は `finish_render` の責務外で、
/// Consumer が別途 `sink.finalize_pdf()` などを呼び出す。
pub trait RenderSink: Send {
    /// 1 ページ確定次第呼ばれる。
    fn accept_page(&mut self, page: PageFragment) -> std::io::Result<()>;

    /// 全 `accept_page` 完了後、`render()` が呼ぶ最終通知。
    ///
    /// `summary` で TargetRegistry の最終状態を Consumer に届け、Consumer は
    /// 未解決 slot を patch する機会を得る (§4.x completion protocol)。
    fn finish_render(&mut self, summary: RenderSummary) -> std::io::Result<()>;
}
```

- [ ] **Step 2: Update `crates/raikiri-traits/src/lib.rs`**

```rust
pub mod sink;
```

```rust
pub use sink::RenderSink;
```

- [ ] **Step 3: Add tests to `lib.rs::tests`**

```rust
    #[test]
    fn render_sink_is_object_safe() {
        fn _assert<T: ?Sized>() {}
        _assert::<dyn RenderSink>();
    }
```

- [ ] **Step 4: Verify build + test pass**

Run: `cargo build -p raikiri-traits && cargo test -p raikiri-traits`
Expected: build green, 12 tests pass。

- [ ] **Step 5: Commit**

```bash
git add crates/raikiri-traits/src/sink.rs crates/raikiri-traits/src/lib.rs
git commit -m "feat(raikiri-traits): add sink module — RenderSink trait (m1.1)"
```

---

## Task 8: strategy.rs — Strategy traits + supporting types

**Files:**
- Create: `crates/raikiri-traits/src/strategy.rs`
- Modify: `crates/raikiri-traits/src/lib.rs`

**Interfaces:**
- Consumes: `sink::RenderSink`, `page::{PageFragment, PageContext}`, `error::TargetKind`, `dom::Symbol`, `std::marker::PhantomData`
- Produces:
  - `pub trait LookaheadPolicy`, `pub trait TargetResolver`, `pub trait EmissionPolicy`, `pub trait ReflowPolicy`
  - `pub struct ProbeContext` (placeholder)
  - `pub struct TargetRequest<'a>` (placeholder), `pub enum ResolvedTarget` (placeholder)
  - `pub enum ReflowAction`, `pub enum ContainerOverflowFallback`, `pub enum DirtyDeadline`

- [ ] **Step 1: Create `crates/raikiri-traits/src/strategy.rs`**

```rust
//! Strategy traits — Streaming と Batch で切り替わる per-render policy。
//!
//! raikiri-dom が各 trait の default 実装群 (BoundedLookahead, UnboundedLookahead,
//! PlaceholderTargetResolver, RegistryTargetResolver, ImmediateEmission,
//! DeferredEmission, AggressiveCommit) を提供する。Consumer は
//! `render_with()` 経由で任意 strategy を組み合わせられる (§4 参照)。

use std::marker::PhantomData;

use crate::error::TargetKind;
use crate::page::{PageContext, PageFragment};
use crate::sink::RenderSink;

/// LayoutBuffer の lookahead 幅を制御する strategy trait。
pub trait LookaheadPolicy {
    /// widow / orphan 判定のため何行先まで探査するか (None = unbounded)。
    fn max_widow_orphan_lines(&self) -> Option<usize>;

    /// `break-inside: avoid` subtree の最大 block 数 (None = unbounded)。
    fn max_break_avoid_subtree_blocks(&self) -> Option<usize>;

    /// flex / grid container の probe layout 上限 (Finding #2 対応)。
    ///
    /// - `None` = unbounded (Batch: container 全体を Fragmentation L3 準拠に layout)
    /// - `Some(N)` = N ページ相当まで、超えたら ReflowPolicy に委譲
    fn max_container_probe_pages(&self) -> Option<usize>;

    /// cross-size (block-progression direction) 方向の lookahead を許可するか。
    fn allow_cross_size_lookahead(&self) -> bool;
}

/// target-* の解決方式 (placeholder emit / 事前 registry lookup)。
pub trait TargetResolver {
    /// 1 target 参照を resolve。
    fn resolve(&mut self, req: TargetRequest<'_>, ctx: &PageContext) -> ResolvedTarget;
}

/// PageFragment の emit タイミング (immediate / deferred)。
pub trait EmissionPolicy {
    /// 1 ページの emit。
    fn emit(&mut self, page: PageFragment, sink: &mut dyn RenderSink) -> std::io::Result<()>;

    /// 全ページ emit 完了通知。
    fn finish(&mut self, sink: &mut dyn RenderSink) -> std::io::Result<()>;
}

/// probe 限界到達時の挙動、および dirty tracking の余地。
///
/// M1〜M8 は `AggressiveCommit` のみ実装、`DirtyDeferred` / `FullReflow` は
/// Future Work (§4 参照)。
pub trait ReflowPolicy {
    /// probe 限界到達時に "即 fallback commit" するか "dirty flag で defer" するか。
    fn on_probe_limit(&self, ctx: &ProbeContext) -> ReflowAction;

    /// dirty tracking を使う場合の memory 上限。
    fn max_dirty_entries(&self) -> Option<usize>;
}

/// ReflowPolicy が返す action。
#[derive(Debug, Clone)]
pub enum ReflowAction {
    /// 即 fallback で commit、取り消し不可 (Streaming preset default)。
    CommitWithFallback(ContainerOverflowFallback),
    /// dirty flag で defer、後続情報で reflow (post-M8 Future Work)。
    DeferAsDirty {
        /// defer の deadline。
        deadline: DirtyDeadline,
    },
}

/// probe 限界時の commit fallback 挙動。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerOverflowFallback {
    /// 次ページに強制配置 (推奨)。
    ForceBreakBefore,
    /// `align-content` 等を無視した単純分割。
    SimpleFragmentation,
    /// 現ページに詰めて overflow。
    OverflowClipping,
    /// fail-loud。
    Error,
}

/// DeferAsDirty の deadline。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirtyDeadline {
    /// 次のページ確定まで defer。
    NextPageBoundary,
    /// 次の container 出現まで defer。
    NextContainerStart,
    /// document 末尾まで defer (Batch preset で活用)。
    DocumentEnd,
}

/// ReflowPolicy が受け取る probe context (M2 で populate)。
///
/// M1.1 では opaque placeholder。
#[allow(missing_docs)]
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct ProbeContext {
    // M2 で populate:
    //   pub node_id: NodeId,
    //   pub probed_pages: u32,
    //   pub container_kind: ContainerKind,
    //   ...
}

impl ProbeContext {
    /// M1.1 placeholder constructor.
    pub fn new() -> Self { Self::default() }
}

/// TargetResolver が受け取る request (M4 で populate)。
///
/// M1.1 では opaque placeholder。
#[allow(missing_docs)]
#[derive(Debug)]
#[non_exhaustive]
pub struct TargetRequest<'a> {
    // M4 で populate:
    //   pub fragment_id: Symbol,
    //   pub kind: TargetKind,
    //   pub source_page: u32,
    _marker: PhantomData<&'a ()>,
}

impl<'a> TargetRequest<'a> {
    /// M1.1 placeholder constructor.
    pub fn new() -> Self {
        Self { _marker: PhantomData }
    }
}

impl<'a> Default for TargetRequest<'a> {
    fn default() -> Self {
        Self::new()
    }
}

/// TargetResolver が返す resolved 情報 (M4 で variant を populate)。
///
/// M1.1 では uninhabited。
#[non_exhaustive]
#[derive(Debug)]
pub enum ResolvedTarget {
    // M4 で populate:
    //   Placeholder { slot_id: TargetSlotId },
    //   Immediate { text: String, kind: TargetKind },
    //   Deferred { fragment_id: Symbol },
    //   Unresolved { fragment_id: Symbol, reason: UnresolvedReason },
}

// TargetKind は placeholder 段階では公開 API に露出しないが、将来 populate 用に
// import。
#[allow(dead_code)]
fn _target_kind_phantom(_k: &TargetKind) {}
```

- [ ] **Step 2: Update `crates/raikiri-traits/src/lib.rs`**

```rust
pub mod strategy;
```

```rust
pub use strategy::{
    ContainerOverflowFallback, DirtyDeadline, EmissionPolicy, LookaheadPolicy, ProbeContext,
    ReflowAction, ReflowPolicy, ResolvedTarget, TargetRequest, TargetResolver,
};
```

- [ ] **Step 3: Add tests to `lib.rs::tests`**

```rust
    #[test]
    fn strategy_placeholders_default_construct() {
        let _ = ProbeContext::default();
        let _ = TargetRequest::default();
    }
```

Note: Strategy trait は generic param として受けるため object-safety 不要 (§4 の
`render_with<L, T, E, R>` 設計参照)。dyn 化は M1.5+ で必要になったら判断。

- [ ] **Step 4: Verify build + test pass**

Run: `cargo build -p raikiri-traits && cargo test -p raikiri-traits`
Expected: build green, 13 tests pass。

- [ ] **Step 5: Commit**

```bash
git add crates/raikiri-traits/src/strategy.rs crates/raikiri-traits/src/lib.rs
git commit -m "feat(raikiri-traits): add strategy module — LookaheadPolicy / TargetResolver / EmissionPolicy / ReflowPolicy (m1.1)"
```

---

## Task 9: config.rs — 5 config structs + 5 Builders

**Files:**
- Create: `crates/raikiri-traits/src/config.rs`
- Modify: `crates/raikiri-traits/src/lib.rs`

**Interfaces:**
- Consumes: `page::TargetRegistry`
- Produces:
  - `pub struct RenderLimits` (#[non_exhaustive]), `pub struct RenderLimitsBuilder`
  - `pub struct LookaheadConfig`, `pub struct LookaheadConfigBuilder`
  - `pub struct StreamingConfig` (#[non_exhaustive]), `pub struct StreamingConfigBuilder`
  - `pub struct BatchConfig` (#[non_exhaustive]), `pub struct BatchConfigBuilder`
  - `pub struct PlanConfig` (#[non_exhaustive]), `pub struct PlanConfigBuilder`

- [ ] **Step 1: Create `crates/raikiri-traits/src/config.rs`**

```rust
//! Render entry point config (Finding #5 対応: raikiri 内 iteration 廃止)。
//!
//! `plan()` / `render_streaming()` / `render_batch()` の 3 entry point が
//! それぞれ config を受け取り、resource / cost limit を強制。round 4 review #1
//! 対応で `RenderLimits` に昇格 (旧 BatchConfig 限定 から plan / Streaming にも
//! 統一)。

use crate::page::TargetRegistry;

/// 全 entry point (plan / render_streaming / render_batch) が受け取る
/// resource / cost 上限 (Finding #5 + round 4 review #1)。
///
/// 妥当な defaults は fulgur 想定: pages=10_000, nodes=1M, slots=100k,
/// buffer=10k, bytes=1GB (§4 §M0 section 参照)。
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct RenderLimits {
    /// 超過 → `LimitExceeded { kind: Pages }`。
    pub max_document_pages: Option<u32>,
    /// parse 完了後 check。
    pub max_dom_nodes: Option<u64>,
    /// per-doc target 参照数上限。
    pub max_target_slots: Option<u32>,
    /// LayoutBuffer に貯める上限。
    pub max_layout_buffer_entries: Option<u32>,
    /// approximate memory footprint 上限。
    pub max_aggregate_bytes: Option<u64>,
}

impl Default for RenderLimits {
    fn default() -> Self {
        Self {
            max_document_pages: Some(10_000),
            max_dom_nodes: Some(1_000_000),
            max_target_slots: Some(100_000),
            max_layout_buffer_entries: Some(10_000),
            max_aggregate_bytes: Some(1_073_741_824), // 1 GB
        }
    }
}

impl RenderLimits {
    /// Default 相当の shortcut。
    pub fn new() -> Self {
        Self::default()
    }

    /// Fluent builder を返す。
    pub fn builder() -> RenderLimitsBuilder {
        RenderLimitsBuilder::default()
    }
}

/// `RenderLimits` の fluent builder。未設定 field は Default 値。
#[derive(Debug, Default, Clone)]
pub struct RenderLimitsBuilder {
    max_document_pages: Option<Option<u32>>,
    max_dom_nodes: Option<Option<u64>>,
    max_target_slots: Option<Option<u32>>,
    max_layout_buffer_entries: Option<Option<u32>>,
    max_aggregate_bytes: Option<Option<u64>>,
}

impl RenderLimitsBuilder {
    /// `max_document_pages` を設定 (`None` = unbounded)。
    pub fn max_document_pages(mut self, v: Option<u32>) -> Self {
        self.max_document_pages = Some(v);
        self
    }

    /// `max_dom_nodes` を設定。
    pub fn max_dom_nodes(mut self, v: Option<u64>) -> Self {
        self.max_dom_nodes = Some(v);
        self
    }

    /// `max_target_slots` を設定。
    pub fn max_target_slots(mut self, v: Option<u32>) -> Self {
        self.max_target_slots = Some(v);
        self
    }

    /// `max_layout_buffer_entries` を設定。
    pub fn max_layout_buffer_entries(mut self, v: Option<u32>) -> Self {
        self.max_layout_buffer_entries = Some(v);
        self
    }

    /// `max_aggregate_bytes` を設定。
    pub fn max_aggregate_bytes(mut self, v: Option<u64>) -> Self {
        self.max_aggregate_bytes = Some(v);
        self
    }

    /// Build。未設定 field は Default 値。
    pub fn build(self) -> RenderLimits {
        let d = RenderLimits::default();
        RenderLimits {
            max_document_pages: self.max_document_pages.unwrap_or(d.max_document_pages),
            max_dom_nodes: self.max_dom_nodes.unwrap_or(d.max_dom_nodes),
            max_target_slots: self.max_target_slots.unwrap_or(d.max_target_slots),
            max_layout_buffer_entries: self
                .max_layout_buffer_entries
                .unwrap_or(d.max_layout_buffer_entries),
            max_aggregate_bytes: self.max_aggregate_bytes.unwrap_or(d.max_aggregate_bytes),
        }
    }
}

/// LayoutBuffer の lookahead 幅 config。
///
/// M1.1 seed value (blitz 慣習ベース、M2/M3 で refine 予定)。
#[derive(Debug, Clone)]
pub struct LookaheadConfig {
    /// widow 判定のため何行先を bufferするか。
    pub widow_line_buffer: usize,
    /// orphan 判定のため何行前を bufferするか。
    pub orphan_line_buffer: usize,
    /// `break-inside: avoid` subtree の最大 block 数。
    pub break_avoid_max_subtree_blocks: usize,
    /// flex / grid container の probe layout 上限 (`None` = unbounded)。
    pub max_container_probe_pages: Option<usize>,
    /// cross-size 方向の lookahead を許可するか。
    pub allow_cross_size_lookahead: bool,
}

impl Default for LookaheadConfig {
    fn default() -> Self {
        // M1.1 seed value, refined in M2/M3.
        Self {
            widow_line_buffer: 2,
            orphan_line_buffer: 2,
            break_avoid_max_subtree_blocks: 20,
            max_container_probe_pages: Some(4),
            allow_cross_size_lookahead: false,
        }
    }
}

impl LookaheadConfig {
    /// Default 相当の shortcut。
    pub fn new() -> Self {
        Self::default()
    }

    /// Fluent builder を返す。
    pub fn builder() -> LookaheadConfigBuilder {
        LookaheadConfigBuilder::default()
    }
}

/// `LookaheadConfig` の fluent builder。
#[derive(Debug, Default, Clone)]
pub struct LookaheadConfigBuilder {
    widow_line_buffer: Option<usize>,
    orphan_line_buffer: Option<usize>,
    break_avoid_max_subtree_blocks: Option<usize>,
    max_container_probe_pages: Option<Option<usize>>,
    allow_cross_size_lookahead: Option<bool>,
}

impl LookaheadConfigBuilder {
    /// `widow_line_buffer` を設定。
    pub fn widow_line_buffer(mut self, v: usize) -> Self {
        self.widow_line_buffer = Some(v);
        self
    }

    /// `orphan_line_buffer` を設定。
    pub fn orphan_line_buffer(mut self, v: usize) -> Self {
        self.orphan_line_buffer = Some(v);
        self
    }

    /// `break_avoid_max_subtree_blocks` を設定。
    pub fn break_avoid_max_subtree_blocks(mut self, v: usize) -> Self {
        self.break_avoid_max_subtree_blocks = Some(v);
        self
    }

    /// `max_container_probe_pages` を設定 (`None` = unbounded)。
    pub fn max_container_probe_pages(mut self, v: Option<usize>) -> Self {
        self.max_container_probe_pages = Some(v);
        self
    }

    /// `allow_cross_size_lookahead` を設定。
    pub fn allow_cross_size_lookahead(mut self, v: bool) -> Self {
        self.allow_cross_size_lookahead = Some(v);
        self
    }

    /// Build。未設定 field は Default 値。
    pub fn build(self) -> LookaheadConfig {
        let d = LookaheadConfig::default();
        LookaheadConfig {
            widow_line_buffer: self.widow_line_buffer.unwrap_or(d.widow_line_buffer),
            orphan_line_buffer: self.orphan_line_buffer.unwrap_or(d.orphan_line_buffer),
            break_avoid_max_subtree_blocks: self
                .break_avoid_max_subtree_blocks
                .unwrap_or(d.break_avoid_max_subtree_blocks),
            max_container_probe_pages: self
                .max_container_probe_pages
                .unwrap_or(d.max_container_probe_pages),
            allow_cross_size_lookahead: self
                .allow_cross_size_lookahead
                .unwrap_or(d.allow_cross_size_lookahead),
        }
    }
}

/// `plan()` 用 config (round 4 review #1, #2 対応)。
#[non_exhaustive]
#[derive(Debug, Default, Clone)]
pub struct PlanConfig {
    /// lookahead 設定。
    pub lookahead: LookaheadConfig,
    /// resource / cost 上限 (round 4 review #1)。
    pub limits: RenderLimits,
    /// 反復 chain 用の hint registry (round 4 review #2)。
    pub initial_registry: Option<TargetRegistry>,
}

impl PlanConfig {
    /// Default 相当の shortcut。
    pub fn new() -> Self {
        Self::default()
    }

    /// Fluent builder を返す。
    pub fn builder() -> PlanConfigBuilder {
        PlanConfigBuilder::default()
    }
}

/// `PlanConfig` の fluent builder。
#[derive(Debug, Default, Clone)]
pub struct PlanConfigBuilder {
    lookahead: Option<LookaheadConfig>,
    limits: Option<RenderLimits>,
    initial_registry: Option<Option<TargetRegistry>>,
}

impl PlanConfigBuilder {
    /// `lookahead` を設定。
    pub fn lookahead(mut self, v: LookaheadConfig) -> Self {
        self.lookahead = Some(v);
        self
    }

    /// `limits` を設定。
    pub fn limits(mut self, v: RenderLimits) -> Self {
        self.limits = Some(v);
        self
    }

    /// `initial_registry` を設定。
    pub fn initial_registry(mut self, v: Option<TargetRegistry>) -> Self {
        self.initial_registry = Some(v);
        self
    }

    /// Build。未設定 field は Default 値。
    pub fn build(self) -> PlanConfig {
        let d = PlanConfig::default();
        PlanConfig {
            lookahead: self.lookahead.unwrap_or(d.lookahead),
            limits: self.limits.unwrap_or(d.limits),
            initial_registry: self.initial_registry.unwrap_or(d.initial_registry),
        }
    }
}

/// `render_streaming()` 用 config (round 4 review #1 対応)。
#[non_exhaustive]
#[derive(Debug, Default, Clone)]
pub struct StreamingConfig {
    /// lookahead 設定。
    pub lookahead: LookaheadConfig,
    /// resource / cost 上限。
    pub limits: RenderLimits,
    /// `plan` の結果を hint として渡す (round 4 review #2)。
    pub initial_registry: Option<TargetRegistry>,
}

impl StreamingConfig {
    /// Default 相当の shortcut。
    pub fn new() -> Self {
        Self::default()
    }

    /// Fluent builder を返す。
    pub fn builder() -> StreamingConfigBuilder {
        StreamingConfigBuilder::default()
    }
}

/// `StreamingConfig` の fluent builder。
#[derive(Debug, Default, Clone)]
pub struct StreamingConfigBuilder {
    lookahead: Option<LookaheadConfig>,
    limits: Option<RenderLimits>,
    initial_registry: Option<Option<TargetRegistry>>,
}

impl StreamingConfigBuilder {
    /// `lookahead` を設定。
    pub fn lookahead(mut self, v: LookaheadConfig) -> Self {
        self.lookahead = Some(v);
        self
    }

    /// `limits` を設定。
    pub fn limits(mut self, v: RenderLimits) -> Self {
        self.limits = Some(v);
        self
    }

    /// `initial_registry` を設定。
    pub fn initial_registry(mut self, v: Option<TargetRegistry>) -> Self {
        self.initial_registry = Some(v);
        self
    }

    /// Build。未設定 field は Default 値。
    pub fn build(self) -> StreamingConfig {
        let d = StreamingConfig::default();
        StreamingConfig {
            lookahead: self.lookahead.unwrap_or(d.lookahead),
            limits: self.limits.unwrap_or(d.limits),
            initial_registry: self.initial_registry.unwrap_or(d.initial_registry),
        }
    }
}

/// `render_batch()` 用 config (round 4 review #1 対応で `max_document_pages` を
/// `limits` に吸収)。
#[non_exhaustive]
#[derive(Debug, Default, Clone)]
pub struct BatchConfig {
    /// resource / cost 上限。
    pub limits: RenderLimits,
    /// `plan` の結果を hint として渡す。
    pub initial_registry: Option<TargetRegistry>,
}

impl BatchConfig {
    /// Default 相当の shortcut。
    pub fn new() -> Self {
        Self::default()
    }

    /// Fluent builder を返す。
    pub fn builder() -> BatchConfigBuilder {
        BatchConfigBuilder::default()
    }
}

/// `BatchConfig` の fluent builder。
#[derive(Debug, Default, Clone)]
pub struct BatchConfigBuilder {
    limits: Option<RenderLimits>,
    initial_registry: Option<Option<TargetRegistry>>,
}

impl BatchConfigBuilder {
    /// `limits` を設定。
    pub fn limits(mut self, v: RenderLimits) -> Self {
        self.limits = Some(v);
        self
    }

    /// `initial_registry` を設定。
    pub fn initial_registry(mut self, v: Option<TargetRegistry>) -> Self {
        self.initial_registry = Some(v);
        self
    }

    /// Build。未設定 field は Default 値。
    pub fn build(self) -> BatchConfig {
        let d = BatchConfig::default();
        BatchConfig {
            limits: self.limits.unwrap_or(d.limits),
            initial_registry: self.initial_registry.unwrap_or(d.initial_registry),
        }
    }
}
```

- [ ] **Step 2: Update `crates/raikiri-traits/src/lib.rs`**

```rust
pub mod config;
```

```rust
pub use config::{
    BatchConfig, BatchConfigBuilder, LookaheadConfig, LookaheadConfigBuilder, PlanConfig,
    PlanConfigBuilder, RenderLimits, RenderLimitsBuilder, StreamingConfig,
    StreamingConfigBuilder,
};
```

- [ ] **Step 3: Add tests to `lib.rs::tests`**

```rust
    #[test]
    fn all_configs_default_construct() {
        let _ = LookaheadConfig::new();
        let _ = LookaheadConfig::default();
        let _ = RenderLimits::default();
        let _ = RenderLimits::new();
        let _ = StreamingConfig::default();
        let _ = BatchConfig::default();
        let _ = PlanConfig::default();
    }

    #[test]
    fn lookahead_config_defaults() {
        let d = LookaheadConfig::default();
        assert_eq!(d.widow_line_buffer, 2);
        assert_eq!(d.orphan_line_buffer, 2);
        assert_eq!(d.break_avoid_max_subtree_blocks, 20);
        assert_eq!(d.max_container_probe_pages, Some(4));
        assert!(!d.allow_cross_size_lookahead);
    }

    #[test]
    fn render_limits_defaults() {
        let d = RenderLimits::default();
        assert_eq!(d.max_document_pages, Some(10_000));
        assert_eq!(d.max_dom_nodes, Some(1_000_000));
        assert_eq!(d.max_target_slots, Some(100_000));
        assert_eq!(d.max_layout_buffer_entries, Some(10_000));
        assert_eq!(d.max_aggregate_bytes, Some(1_073_741_824));
    }

    #[test]
    fn lookahead_config_builder_roundtrip() {
        let cfg = LookaheadConfig::builder()
            .widow_line_buffer(5)
            .max_container_probe_pages(None)
            .build();
        assert_eq!(cfg.widow_line_buffer, 5);
        assert_eq!(cfg.max_container_probe_pages, None);
        // 未設定 field は default 値
        assert_eq!(cfg.orphan_line_buffer, LookaheadConfig::default().orphan_line_buffer);
    }

    #[test]
    fn render_limits_builder_roundtrip() {
        let cfg = RenderLimits::builder()
            .max_document_pages(Some(100))
            .max_dom_nodes(None)
            .build();
        assert_eq!(cfg.max_document_pages, Some(100));
        assert_eq!(cfg.max_dom_nodes, None);
        // 未設定 field は default 値
        assert_eq!(cfg.max_target_slots, RenderLimits::default().max_target_slots);
    }

    #[test]
    fn streaming_config_builder_roundtrip() {
        let limits = RenderLimits::builder().max_document_pages(Some(50)).build();
        let cfg = StreamingConfig::builder().limits(limits).build();
        assert_eq!(cfg.limits.max_document_pages, Some(50));
    }
```

- [ ] **Step 4: Verify build + test pass**

Run: `cargo build -p raikiri-traits && cargo test -p raikiri-traits`
Expected: build green, 19 tests pass。

- [ ] **Step 5: Commit**

```bash
git add crates/raikiri-traits/src/config.rs crates/raikiri-traits/src/lib.rs
git commit -m "feat(raikiri-traits): add config module — RenderLimits / LookaheadConfig / Streaming / Batch / Plan (m1.1)"
```

---

## Task 10: plan.rs — DocumentPlan + supporting types

**Files:**
- Create: `crates/raikiri-traits/src/plan.rs`
- Modify: `crates/raikiri-traits/src/lib.rs`

**Interfaces:**
- Consumes: `page::{PageBox, TargetRegistry}`, `error::UnresolvedTarget`
- Produces:
  - `pub struct DocumentPlan`
  - `pub struct PageSummary`
  - `pub struct TargetDefinition` (#[non_exhaustive], placeholder)
  - `pub enum BreakReason` (#[non_exhaustive], placeholder)

- [ ] **Step 1: Create `crates/raikiri-traits/src/plan.rs`**

```rust
//! `plan()` (dry-run) 用の output 型。
//!
//! Finding #5 対応: raikiri は自身で iteration しない。`plan` は 1 pass の
//! `DocumentPlan` を返し、Consumer が自身の convergence loop で hint として
//! 再入力する。

use crate::error::UnresolvedTarget;
use crate::page::{PageBox, TargetRegistry};

/// `plan()` の output (paint scene / PaintedBox 構築なし)。
///
/// 用途: fulgur の pass-1 前哨、cost 見積り、target 収束判定用の hint 生成。
#[derive(Debug)]
pub struct DocumentPlan {
    /// 総ページ数。
    pub total_pages: u32,
    /// hint: render 時は再計算される (Finding #5)。
    pub target_registry: TargetRegistry,
    /// target 定義 list (M4 で populate)。
    pub target_definitions: Vec<TargetDefinition>,
    /// 未解決 target list。
    pub unresolved_targets: Vec<UnresolvedTarget>,
    /// 各ページの summary (box tree, glyph 情報なし)。
    pub page_summary: Vec<PageSummary>,
}

/// 1 ページの summary (`plan()` output の要素)。
#[derive(Debug)]
pub struct PageSummary {
    /// 0-indexed page number。
    pub page_index: u32,
    /// このページの実効 `@page` 解決結果。
    pub page_box: PageBox,
    /// なぜこのページで break したか。
    pub break_reason: BreakReason,
    /// このページに含まれる target slot 数。
    pub target_slot_count: u32,
    /// このページで新規定義された target 数。
    pub target_definition_count: u32,
    /// このページの content 高さ (physical pixel)。
    pub content_height: f32,
}

/// `page-break-*` / auto-fill 等、ページ break の理由 (M2 で variant を populate)。
///
/// M1.1 では uninhabited。想定 variant:
///   - `PageBreakBefore { property: BreakProperty }`
///   - `PageBreakAfter { property: BreakProperty }`
///   - `PageBreakInside`
///   - `AutoFill { content_height: f32, page_height: f32 }`
///   - `ExplicitBreakElement { node_id: NodeId }`
#[non_exhaustive]
#[derive(Debug)]
pub enum BreakReason {
    // M2 で populate。
}

/// target-* の定義側情報 (`target-counter` などが参照する source location)。
///
/// M4 で fields を populate。M1.1 では opaque。
#[allow(missing_docs)]
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct TargetDefinition {
    // M4 で populate:
    //   pub fragment_id: Symbol,
    //   pub page_index: u32,
    //   pub kind: TargetKind,
    //   pub text: String,
    //   ...
}

impl TargetDefinition {
    /// M1.1 placeholder constructor.
    pub fn new() -> Self {
        Self::default()
    }
}
```

- [ ] **Step 2: Update `crates/raikiri-traits/src/lib.rs`**

```rust
pub mod plan;
```

```rust
pub use plan::{BreakReason, DocumentPlan, PageSummary, TargetDefinition};
```

- [ ] **Step 3: Add tests to `lib.rs::tests`**

```rust
    #[test]
    fn plan_placeholder_default_construct() {
        let _ = TargetDefinition::default();
    }
```

- [ ] **Step 4: Verify build + test pass**

Run: `cargo build -p raikiri-traits && cargo test -p raikiri-traits`
Expected: build green, 20 tests pass。

- [ ] **Step 5: Commit**

```bash
git add crates/raikiri-traits/src/plan.rs crates/raikiri-traits/src/lib.rs
git commit -m "feat(raikiri-traits): add plan module — DocumentPlan + PageSummary (m1.1)"
```

---

## Task 11: lib.rs — Final re-exports, doc, comprehensive tests

**Files:**
- Modify: `crates/raikiri-traits/src/lib.rs`

**Interfaces:**
- Consumes: 全 submodule
- Produces: crate-level docs, `pub use` re-exports, comprehensive `#[cfg(test)]` tests

- [ ] **Step 1: Rewrite `crates/raikiri-traits/src/lib.rs` to a comprehensive shape**

現状の incremental な lib.rs を、以下 comprehensive version に置き換える (全 mod 宣言 + 主要型 re-export + 完全 test suite):

```rust
//! raikiri-traits — foundation traits and neutral model types.
//!
//! 設計仕様書 §4 に定義された全 trait / 中立モデル型を集約する。実装は持たず、
//! raikiri-html / raikiri-style / raikiri-dom / raikiri-paint / raikiri-net が
//! 参照する共通型層。
//!
//! ## Module tour
//!
//! - [`dom`]      — DOM abstraction trait + identifier newtypes (Symbol, NodeId)
//! - [`page`]     — Page-related opaque model types (PageFragment, PageBox, ...)
//! - [`policy`]   — ResourcePolicy trait + violation types
//! - [`net`]      — NetworkProvider trait + Request / FetchedResource types
//! - [`resolver`] — ReplacedResolver trait + intrinsic size types
//! - [`error`]    — RenderError taxonomy + status / summary types
//! - [`sink`]     — RenderSink trait
//! - [`strategy`] — Strategy traits (LookaheadPolicy, TargetResolver, EmissionPolicy, ReflowPolicy)
//! - [`config`]   — Entry point configs (RenderLimits, LookaheadConfig, ...)
//! - [`plan`]     — `plan()` output types (DocumentPlan, PageSummary)
//!
//! ## Spec authority
//!
//! 型定義の authoritative source は
//! `docs/superpowers/specs/2026-07-13-raikiri-rebuild-design.md` §4 (§13.0 spec
//! drift protocol 準拠)。narrative §5-§12 は補助資料。

pub mod config;
pub mod dom;
pub mod error;
pub mod net;
pub mod page;
pub mod plan;
pub mod policy;
pub mod resolver;
pub mod sink;
pub mod strategy;

// 主要型 crate-root re-export (Consumer が `use raikiri_traits::*` で足りる shape)
pub use config::{
    BatchConfig, BatchConfigBuilder, LookaheadConfig, LookaheadConfigBuilder, PlanConfig,
    PlanConfigBuilder, RenderLimits, RenderLimitsBuilder, StreamingConfig,
    StreamingConfigBuilder,
};
pub use dom::{Dom, Element, Node, NodeId, Symbol};
pub use error::{
    CascadeError, EmittedSlotInfo, ExhaustionPolicy, LayoutError, LimitKind, ParseError,
    RenderError, RenderStatus, RenderSummary, RenderWarning, TargetDiscrepancy, TargetKind,
    TargetSlotId, UnresolvedReason, UnresolvedTarget, WarningKind,
};
pub use net::{
    AbortController, AbortSignal, Body, FetchedResource, HeaderMap, Method, NetworkError,
    NetworkProvider, Request,
};
pub use page::{
    ContentValueItem, FormData, GcpmDirective, LayoutBuffer, PageBox, PageContext,
    PageFragment, RunningTemplate, TargetRegistry,
};
pub use plan::{BreakReason, DocumentPlan, PageSummary, TargetDefinition};
pub use policy::{PolicyViolation, ResourceKind, ResourcePolicy, ViolationType};
pub use resolver::{
    IntrinsicBox, ReplacedResolver, ResolveDisposition, ResolvedIntrinsic, ResolverError,
    ResolverRequest,
};
pub use sink::RenderSink;
pub use strategy::{
    ContainerOverflowFallback, DirtyDeadline, EmissionPolicy, LookaheadPolicy, ProbeContext,
    ReflowAction, ReflowPolicy, ResolvedTarget, TargetRequest, TargetResolver,
};

#[cfg(test)]
mod tests {
    use super::*;

    // ── DOM foundations ─────────────────────────────────────────

    #[test]
    fn symbol_construct_from_str() {
        let s = Symbol::from("target-1");
        assert_eq!(s.as_str(), "target-1");
    }

    #[test]
    fn nodeid_construct() {
        let n = NodeId::new(42);
        assert_eq!(n.0, 42);
    }

    // ── Page placeholders ───────────────────────────────────────

    #[test]
    fn page_placeholders_default_construct() {
        let _ = PageFragment::default();
        let _ = PageFragment::new();
        let _ = PageBox::default();
        let _ = PageContext::default();
        let _ = LayoutBuffer::default();
        let _ = TargetRegistry::default();
        let _ = RunningTemplate::default();
        let _ = FormData::default();
    }

    // ── Trait object safety ─────────────────────────────────────

    #[test]
    fn dyn_traits_are_object_safe() {
        fn _assert<T: ?Sized>() {}
        _assert::<dyn RenderSink>();
        _assert::<dyn ReplacedResolver>();
        _assert::<dyn NetworkProvider>();
        _assert::<dyn ResourcePolicy>();
        // Strategy traits (LookaheadPolicy / TargetResolver / EmissionPolicy /
        // ReflowPolicy) は generic param 経由で受ける (§4 `render_with<L,T,E,R>`
        // 設計) ため object-safety は要件外。M1.5+ で dyn 化が必要なら判断。
        //
        // Dom / Element / Node は M1.5 dom-model で associated type / GAT を
        // 追加する予定で、その段階で non-object-safe になる可能性が高いため
        // M1.1 では assert しない。
    }

    // ── AbortController semantic ────────────────────────────────

    #[test]
    fn abort_controller_default_and_abort() {
        let c = AbortController::new();
        assert!(!c.signal.is_aborted());
        c.abort();
        assert!(c.signal.is_aborted());
    }

    // ── Resolver placeholders ───────────────────────────────────

    #[test]
    fn resolver_placeholder_types_default_construct() {
        let _ = IntrinsicBox::default();
        let _ = ResolverRequest::default();
    }

    // ── Error taxonomy ──────────────────────────────────────────

    #[test]
    fn render_error_is_error_trait() {
        fn _assert<T: std::error::Error>() {}
        _assert::<RenderError>();
    }

    #[test]
    fn exhaustion_policy_default_is_error() {
        assert_eq!(ExhaustionPolicy::default(), ExhaustionPolicy::Error);
    }

    #[test]
    fn target_slot_id_construct() {
        let id = TargetSlotId { page_index: 3, sequence: 7 };
        assert_eq!(id.page_index, 3);
        assert_eq!(id.sequence, 7);
    }

    // ── Strategy placeholders ───────────────────────────────────

    #[test]
    fn strategy_placeholders_default_construct() {
        let _ = ProbeContext::default();
        let _ = TargetRequest::default();
    }

    // ── Config defaults ─────────────────────────────────────────

    #[test]
    fn all_configs_default_construct() {
        let _ = LookaheadConfig::new();
        let _ = LookaheadConfig::default();
        let _ = RenderLimits::default();
        let _ = RenderLimits::new();
        let _ = StreamingConfig::default();
        let _ = BatchConfig::default();
        let _ = PlanConfig::default();
    }

    #[test]
    fn lookahead_config_defaults() {
        let d = LookaheadConfig::default();
        assert_eq!(d.widow_line_buffer, 2);
        assert_eq!(d.orphan_line_buffer, 2);
        assert_eq!(d.break_avoid_max_subtree_blocks, 20);
        assert_eq!(d.max_container_probe_pages, Some(4));
        assert!(!d.allow_cross_size_lookahead);
    }

    #[test]
    fn render_limits_defaults() {
        let d = RenderLimits::default();
        assert_eq!(d.max_document_pages, Some(10_000));
        assert_eq!(d.max_dom_nodes, Some(1_000_000));
        assert_eq!(d.max_target_slots, Some(100_000));
        assert_eq!(d.max_layout_buffer_entries, Some(10_000));
        assert_eq!(d.max_aggregate_bytes, Some(1_073_741_824));
    }

    // ── Config builders ─────────────────────────────────────────

    #[test]
    fn lookahead_config_builder_roundtrip() {
        let cfg = LookaheadConfig::builder()
            .widow_line_buffer(5)
            .max_container_probe_pages(None)
            .build();
        assert_eq!(cfg.widow_line_buffer, 5);
        assert_eq!(cfg.max_container_probe_pages, None);
        assert_eq!(cfg.orphan_line_buffer, LookaheadConfig::default().orphan_line_buffer);
    }

    #[test]
    fn render_limits_builder_roundtrip() {
        let cfg = RenderLimits::builder()
            .max_document_pages(Some(100))
            .max_dom_nodes(None)
            .build();
        assert_eq!(cfg.max_document_pages, Some(100));
        assert_eq!(cfg.max_dom_nodes, None);
        assert_eq!(cfg.max_target_slots, RenderLimits::default().max_target_slots);
    }

    #[test]
    fn streaming_config_builder_roundtrip() {
        let limits = RenderLimits::builder().max_document_pages(Some(50)).build();
        let cfg = StreamingConfig::builder().limits(limits).build();
        assert_eq!(cfg.limits.max_document_pages, Some(50));
    }

    #[test]
    fn batch_config_builder_roundtrip() {
        let limits = RenderLimits::builder().max_document_pages(Some(30)).build();
        let cfg = BatchConfig::builder().limits(limits).build();
        assert_eq!(cfg.limits.max_document_pages, Some(30));
    }

    #[test]
    fn plan_config_builder_roundtrip() {
        let lookahead = LookaheadConfig::builder().widow_line_buffer(3).build();
        let cfg = PlanConfig::builder().lookahead(lookahead).build();
        assert_eq!(cfg.lookahead.widow_line_buffer, 3);
    }

    // ── Plan placeholders ───────────────────────────────────────

    #[test]
    fn plan_placeholder_default_construct() {
        let _ = TargetDefinition::default();
    }
}
```

- [ ] **Step 2: Verify build + test pass**

Run: `cargo build -p raikiri-traits && cargo test -p raikiri-traits`
Expected: build green, 21 tests pass。

- [ ] **Step 3: Commit**

```bash
git add crates/raikiri-traits/src/lib.rs
git commit -m "feat(raikiri-traits): finalize lib.rs — crate doc + comprehensive tests (m1.1)"
```

---

## Task 12: Acceptance verification + lint pass + close

**Files:**
- None (verification only)

**Interfaces:**
- Consumes: 全 submodule
- Produces: acceptance verdict + beads issue close

- [ ] **Step 1: Full-workspace build to catch consumer breakage**

Run: `cargo build --workspace`
Expected: green (raikiri-traits の変更が他 crate を break していない)。

- [ ] **Step 2: Full-workspace test**

Run: `cargo test --workspace`
Expected: 既存 test は全 pass + raikiri-traits の新 test 21 個 pass。

- [ ] **Step 3: Clippy pass (workspace-strict lint)**

Run: `cargo clippy -p raikiri-traits -- -D warnings`
Expected: warning 0 で pass。

- [ ] **Step 4: Doc build check**

Run: `cargo doc -p raikiri-traits --no-deps`
Expected: warning 0 で pass (missing_docs も検証)。

- [ ] **Step 5: Acceptance checklist verification**

以下チェックリストを 1 項目ずつ突き合わせ、全 tick で close 可:

- [ ] `cargo build -p raikiri-traits` green
- [ ] `cargo test -p raikiri-traits` 21 test pass
- [ ] `cargo clippy -p raikiri-traits -- -D warnings` pass
- [ ] `cargo doc -p raikiri-traits --no-deps` pass
- [ ] `#[non_exhaustive]` pub struct 全て `Default::default()` construct 可能:
      PageFragment, PageBox, PageContext, LayoutBuffer, TargetRegistry, RunningTemplate,
      FormData, IntrinsicBox, ResolverRequest, ProbeContext, TargetRequest, TargetDefinition,
      RenderLimits, StreamingConfig, BatchConfig, PlanConfig (LookaheadConfig は #[non_exhaustive]
      なし — 全 field 完全定義済)
- [ ] Trait 全て populate:
      RenderSink, ReplacedResolver, NetworkProvider, ResourcePolicy, LookaheadPolicy,
      TargetResolver, EmissionPolicy, ReflowPolicy (+ Dom / Element / Node は shell)
- [ ] Config 5 種 + Builder 5 種: RenderLimits, LookaheadConfig, StreamingConfig, BatchConfig,
      PlanConfig
- [ ] Error 型: RenderError (11 variant), RenderStatus, RenderSummary, RenderWarning,
      WarningKind, ExhaustionPolicy, LimitKind, ParseError, CascadeError, LayoutError,
      UnresolvedTarget, UnresolvedReason, EmittedSlotInfo, TargetSlotId, TargetKind
- [ ] Networking: Request, FetchedResource, Body, Method, AbortSignal, AbortController,
      NetworkError, HeaderMap, FormData
- [ ] Policy: ResourceKind, PolicyViolation, ViolationType
- [ ] Plan: DocumentPlan, PageSummary, TargetDefinition, TargetDiscrepancy, BreakReason
- [ ] Finding number (`Finding #N` / `round X review #Y`) が doc に転記されている

- [ ] **Step 6: Record m1.2 boundary shift in bd memory**

Run:
```bash
bd remember "m1.2-scope-shift" "m1.1 closed: raikiri-traits に error taxonomy 型 (ParseError / CascadeError / LayoutError / ResolverError / RenderError / RenderStatus etc.) を含めて全 populate 済。m1.2 の実 scope は raikiri-html (M1.3) / raikiri-style (M1.4) からの RenderError::Parse / Cascade 配線 + ParseError / CascadeError / LayoutError variant populate + RenderStatus::Aborted の partial_pages 追跡 verify test に shift。m1.2 close 前に title / description を bd update で調整すること。"
```

- [ ] **Step 7: Optionally update m1.2 title/description**

以下は m1.1 close の一部として推奨 (m1.2 実施者への signal):

```bash
bd update raikiri-spike-m1.2 \
  --title="error-propagation-wiring: RenderError bubble up from Parse/Cascade + variant populate" \
  --description="m1.1 で raikiri-traits に定義済の RenderError / RenderStatus / ParseError / CascadeError / LayoutError の下流配線タスク。(1) raikiri-html (M1.3) から RenderError::Parse(ParseError::...) の bubble up 実配線 + variant populate、(2) raikiri-style (M1.4) から RenderError::Cascade(CascadeError::...) 同、(3) RenderStatus::Aborted の partial_pages 追跡の verify test。設計仕様書 §M1 Goals #3、Task #H1 対応。"
```

- [ ] **Step 8: Final commit + branch state check**

```bash
git status
git log --oneline main..HEAD
```
Expected: 上記 11 個の feat commit が並ぶ、working tree clean。

- [ ] **Step 9: Confirm blueprint:impl completion protocol**

M1.1 の deliverable が全て commit 済み。次は blueprint:impl の Step 6 (verification + finishing-a-development-branch)。

---

## Self-Review Checklist (plan writer 側)

- **Spec coverage**: §4 raikiri-traits 記述の全 trait / struct / enum が Task 1-11 に mapped。
  - Category A (fully populated) 型: Task 6 (error taxonomy)、Task 9 (5 configs)、Task 10 (plan types) にカバー。
  - Category B (trait) 型: Task 3 (ResourcePolicy)、Task 4 (NetworkProvider)、Task 5 (ReplacedResolver)、Task 7 (RenderSink)、Task 8 (strategy 4 traits) にカバー。
  - Category C (opaque placeholder) 型: Task 1 (Dom / Element / Node)、Task 2 (page.rs 9 types)、Task 5 (IntrinsicBox / ResolverRequest / ResolverError)、Task 6 (ParseError / CascadeError / LayoutError / TargetKind placeholder)、Task 8 (ProbeContext / TargetRequest / ResolvedTarget)、Task 10 (TargetDefinition / BreakReason) にカバー。
- **Placeholder scan**: "TBD" / "TODO" / "implement later" 等の禁止語句なし。placeholder 型は明示的に "M1.1 placeholder, populated in Mx.y" コメント + `#[allow(missing_docs)]`。
- **Type consistency**: Task 間で型名が一致していることを verify (RenderError, TargetKind, PageFragment 等)。Symbol は dom.rs で定義、Task 6, 9 が use。
- **Missing spec item**: §4 の全 pub item が Task に対応。§5, §7, §9, §11 の詳細 (PageFragment fields, PageContext state machine 等) は M1.1 non-goals として明示 (design doc に記載済)。

---

## Execution Notes

- **branch**: 全 commit は `raikiri-spike-m1.1` branch (`.claude/worktrees/raikiri-spike-m1.1/`)。
- **commit granularity**: 1 module 1 commit の TDD 相当 (submodule ごとに build + test 通過を確認)。
- **CLAUDE.md 準拠**: git push / dolt sync は本 plan 内では行わない (conservative profile default)。task 完了後に user が明示的に指示した場合のみ。
- **rollback**: 各 task 独立に revert 可能。task N の commit を revert しても、task 1 ~ (N-1) の crate 状態は intact。
