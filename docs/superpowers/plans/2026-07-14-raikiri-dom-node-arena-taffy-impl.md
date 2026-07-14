# raikiri-dom node arena + taffy trait impl Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `crates/raikiri-dom/` に node arena + taffy 6 trait impl + raikiri-traits::Dom/Element/Node の minimum-viable co-design を実装 (feasibility-report §3.2 の SpikeTree pattern を production 化)。

**Architecture:** `Document` は Vec-backed arena (`nodes: Vec<Node>`) を持ち、taffy の 6 low-level trait を直接 impl。raikiri-traits に定義された `Dom / Node / Element` trait は M1.5 で GAT + minimum-viable method に拡充される (M1.1 で shell だった箇所を co-design)。calc feature の Send 問題は spike 由来の `unsafe impl Send` で暫定処理、m1.17 で最終確定。

**Tech Stack:** Rust 2024 edition, MSRV 1.89, taffy 0.12 (features: block_layout, flexbox, grid, content_size, calc, std)、smol_str, raikiri-traits (workspace dep)。

## Global Constraints

- **Authoritative spec section**: `docs/superpowers/specs/2026-07-13-raikiri-rebuild-design.md` §4 (raikiri-dom 記述) と §M1 acceptance criteria。
- **Feasibility material**: `docs/feasibility-report.md` §3.2 (SpikeTree pattern)。実装原典は `crates/raikiri-feasibility/src/taffy_layout_modes.rs`。
- **beads issue**: `raikiri-spike-m1.5`。design + acceptance フィールド保存済。
- **workspace lints**: `unsafe_code = "deny"`, `missing_docs = "warn"`。`unsafe impl Send for Document {}` は 1 箇所のみ `#[allow(unsafe_code)]` で局所許可、SAFETY doc comment で invariant を明示。
- **taffy features**: block_layout, flexbox, grid, content_size, calc, std (workspace dep で確定済、変更不要)。`taffy_tree` feature は OFF (raikiri は自 arena に trait 直下 impl)。
- **raikiri-traits patch**: M1.5 branch 内で M1.1 の shell trait を expand する commit を 1 個作る (Task 1)。M1.1 の 19 tests が break しないこと。
- **Branch**: `raikiri-spike-m1.5` (worktree `.claude/worktrees/raikiri-spike-m1.5/`)。
- **Verification per task**: 各 task 末尾で `cargo build -p <affected>`, `cargo test -p <affected>`, `cargo fmt -p <affected> -- --check` green。
- **Final verification (Task 7)**: `cargo build -p raikiri-dom`, `cargo test -p raikiri-dom`, `cargo clippy -p raikiri-dom -- -D warnings`, `cargo fmt -p raikiri-dom -- --check`, `cargo build --workspace`, `cargo test -p raikiri-traits` (19 tests intact) 全 pass。

---

## File Structure

- Modify: `crates/raikiri-traits/src/dom.rs` — Dom / Node / Element trait を M1.1 shell から minimum-viable に populate (NodeKind enum + 4 methods + GAT)
- Modify: `crates/raikiri-traits/src/lib.rs` — `pub use dom::NodeKind` 追加
- Create: `crates/raikiri-dom/src/node.rs` — Node struct + kind / tag / text field
- Create: `crates/raikiri-dom/src/document.rs` — Document arena + append API
- Create: `crates/raikiri-dom/src/taffy_impl.rs` — taffy 6 trait impl + `unsafe impl Send for Document {}`
- Create: `crates/raikiri-dom/src/dom_impl.rs` — raikiri_traits Dom / Node / Element impl
- Modify: `crates/raikiri-dom/src/lib.rs` — M0 stub を書き換え、pub mod + re-export + tests

**Dependency order (Task 番号順に build)**:
1. raikiri-traits patch (Task 1) — Dom/Node/Element を expand
2. node.rs (Task 2) — Node struct、standalone
3. document.rs (Task 3) — Document arena、Node に依存
4. taffy_impl.rs (Task 4) — Document に taffy trait を impl
5. dom_impl.rs (Task 5) — Document / NodeRef / ElementRef に raikiri-traits を impl
6. lib.rs 最終形 (Task 6) — pub mod + pub use + tests
7. Acceptance verification (Task 7)

---

## Task 1: raikiri-traits patch — Dom / Node / Element trait を populate

**Files:**
- Modify: `crates/raikiri-traits/src/dom.rs`
- Modify: `crates/raikiri-traits/src/lib.rs`

**Interfaces:**
- Consumes: 既存の `NodeId`, `Symbol`
- Produces:
  - `pub enum NodeKind { Element, Text, Document }`
  - `pub trait Dom` with `type NodeRef<'a>`, `type ElementRef<'a>`, `type ChildIter<'a>`, 3 methods
  - `pub trait Node<'a>` with `type Element<'b>`, 3 methods
  - `pub trait Element<'a>` with 1 method

- [ ] **Step 1: Modify `crates/raikiri-traits/src/dom.rs`**

現状 (M1.1 shell 部分) を置き換え。ファイル全体は既存 (Symbol, NodeId 定義は保持)、下記の trait ブロックだけ置換:

置換対象 (M1.1 の shell trait コメント block):
```rust
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

新しい内容 (M1.5 populate):
```rust
/// DOM node の種別 (Element / Text / Document root)。
///
/// M1.5 で raikiri-dom node arena の kind field と対応する。将来 (M4)
/// Comment / CDATA / ProcessingInstruction 等が加わる可能性があるため
/// `#[non_exhaustive]`。
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeKind {
    /// HTML / XML element (tag_name あり)。
    Element,
    /// Character data node。
    Text,
    /// Document root (arena index 0 に配置される仮想 node)。
    Document,
}

/// DOM tree abstraction。raikiri-dom / raikiri-style / raikiri-paint / raikiri
/// (umbrella) が消費する generic navigation interface。
///
/// **Object-safety**: GAT (`type NodeRef<'a>`) を含むため non-object-safe。
/// M1 では generic dispatch (`fn walk<D: Dom>(dom: &D)`) を前提。dyn 化が
/// 必要な場合 (M6 blitz-compat 経由の runtime abstraction 等) は erased
/// wrapper trait を別途用意する。
pub trait Dom {
    /// Node reference (borrowed) type。
    type NodeRef<'a>: Node<'a>
    where
        Self: 'a;
    /// Element reference (borrowed) type。
    type ElementRef<'a>: Element<'a>
    where
        Self: 'a;
    /// Child ID iterator type。
    type ChildIter<'a>: Iterator<Item = NodeId>
    where
        Self: 'a;

    /// Document root node の identifier。実装は通常 arena index 0 の Document
    /// kind node を指す。
    fn root_id(&self) -> NodeId;

    /// `id` に対応する Node reference。範囲外なら `None`。
    fn node(&self, id: NodeId) -> Option<Self::NodeRef<'_>>;

    /// `id` の direct children を走査する iterator。
    fn child_ids(&self, id: NodeId) -> Self::ChildIter<'_>;
}

/// Node reference (borrowed lifetime `'a`)。kind ごとの dispatch と共通 API を
/// 提供。
pub trait Node<'a> {
    /// Element downcast 用の Element reference type。
    type Element<'b>: Element<'b>
    where
        Self: 'b;

    /// この node の種別。
    fn kind(&self) -> NodeKind;

    /// kind が Element の場合 Element reference を返す。それ以外 (Text /
    /// Document) は `None`。
    fn as_element(&self) -> Option<Self::Element<'_>>;

    /// kind が Text の場合 character data。それ以外 (Element / Document) は
    /// `None`。
    fn text_content(&self) -> Option<&str>;
}

/// Element reference (borrowed lifetime `'a`)。
///
/// M1.6 以降で attribute / classes / id lookup 等を追加する予定。M1.5 は
/// tag_name のみ確定。
pub trait Element<'a> {
    /// HTML / XML tag name (例: `"p"`, `"div"`)。
    fn tag_name(&self) -> &str;
}
```

- [ ] **Step 2: Modify `crates/raikiri-traits/src/lib.rs`**

`pub use dom::{...}` の import list に `NodeKind` を追加:

置換対象:
```rust
pub use dom::{Dom, Element, Node, NodeId, Symbol};
```

新しい内容:
```rust
pub use dom::{Dom, Element, Node, NodeId, NodeKind, Symbol};
```

- [ ] **Step 3: Verify raikiri-traits build + tests still pass**

Run: `cargo build -p raikiri-traits && cargo test -p raikiri-traits`
Expected: build clean, **19 tests pass** (M1.1 の tests が break しない)。特に `dyn_traits_are_object_safe` は Dom を含めていないので GAT 追加後も OK。

Run: `cargo fmt -p raikiri-traits -- --check`
Expected: clean。

- [ ] **Step 4: Commit**

```bash
cd .claude/worktrees/raikiri-spike-m1.5
git add crates/raikiri-traits/src/dom.rs crates/raikiri-traits/src/lib.rs
git commit -m "feat(raikiri-traits): populate Dom / Node / Element trait for M1.5 co-design

M1.1 の shell から minimum-viable な GAT + 4 method へ拡充:
- NodeKind (Element / Text / Document、#[non_exhaustive])
- Dom { type NodeRef<'a>, ElementRef<'a>, ChildIter<'a>; root_id, node, child_ids }
- Node<'a> { type Element<'b>; kind, as_element, text_content }
- Element<'a> { tag_name }

GAT により non-object-safe になるが M1 は generic dispatch 前提のため OK。
既存 19 tests は不変。M1.5 raikiri-dom がこの trait を Document 上に実装する。

Claude-Session: https://claude.ai/code/session_01Qo7TYgbGc4JQnghRS6nUJe"
```

---

## Task 2: raikiri-dom node.rs — Node struct

**Files:**
- Create: `crates/raikiri-dom/src/node.rs`

**Interfaces:**
- Consumes: `taffy::{Style, Cache, Layout}`, `smol_str::SmolStr`, `raikiri_traits::NodeKind`
- Produces: `pub(crate) struct Node` with 7 pub(crate) fields; `pub(crate) fn new_document / new_element / new_text` constructors

- [ ] **Step 1: Create `crates/raikiri-dom/src/node.rs`**

```rust
//! Arena node type for raikiri-dom's Document.
//!
//! Node は Element / Text / Document (root) の 3 種を union で表現する
//! flat struct。fields は crate-private (raikiri-dom 内部のみ mutate)、
//! 外部 Consumer は `raikiri_traits::Dom / Node / Element` trait 経由で
//! 参照する。

use smol_str::SmolStr;
use taffy::{Cache, Layout, Style};

use raikiri_traits::NodeKind;

/// Arena node。全 field は crate-private。
///
/// M1.5 では `style` を Consumer が taffy::Style 直接構築する形。M1.6
/// layout-single-page で ComputedValues → taffy::Style 変換 layer が入る予定。
pub(crate) struct Node {
    /// Taffy layout style。
    pub(crate) style: Style,
    /// Child arena indices (`Document::nodes` の usize)。
    pub(crate) children: Vec<usize>,
    /// Taffy layout cache (per-node)。
    pub(crate) cache: Cache,
    /// Taffy layout 結果 (compute_root_layout が populate)。
    pub(crate) unrounded_layout: Layout,
    /// Node kind (Element / Text / Document)。
    pub(crate) kind: NodeKind,
    /// Element tag name (kind == Element 時のみ populate、他は `None`)。
    pub(crate) tag_name: Option<SmolStr>,
    /// Text character data (kind == Text 時のみ populate、他は `None`)。
    pub(crate) text_content: Option<SmolStr>,
}

impl Node {
    /// Document root node (arena index 0 用) を構築する。
    pub(crate) fn new_document() -> Self {
        Self {
            style: Style::default(),
            children: Vec::new(),
            cache: Cache::new(),
            unrounded_layout: Layout::with_order(0),
            kind: NodeKind::Document,
            tag_name: None,
            text_content: None,
        }
    }

    /// Element node を tag name と style と共に構築する。
    pub(crate) fn new_element(tag: SmolStr, style: Style) -> Self {
        Self {
            style,
            children: Vec::new(),
            cache: Cache::new(),
            unrounded_layout: Layout::with_order(0),
            kind: NodeKind::Element,
            tag_name: Some(tag),
            text_content: None,
        }
    }

    /// Text node を character data と共に構築する。
    pub(crate) fn new_text(text: SmolStr) -> Self {
        Self {
            style: Style::default(),
            children: Vec::new(),
            cache: Cache::new(),
            unrounded_layout: Layout::with_order(0),
            kind: NodeKind::Text,
            tag_name: None,
            text_content: Some(text),
        }
    }
}
```

- [ ] **Step 2: Verify compile-only**

Since node.rs isn't wired into lib.rs yet, `cargo build -p raikiri-dom` won't include this file. Skip build for this task; Task 6 wires everything up.

However, verify no syntax errors by running:
Run: `cargo check -p raikiri-dom` — expected: build (existing stub) unaffected (module not yet declared).

- [ ] **Step 3: Commit**

```bash
git add crates/raikiri-dom/src/node.rs
git commit -m "feat(raikiri-dom): add Node arena element (m1.5)

Node struct with taffy::Style / children / cache / kind / tag / text
fields (all pub(crate)). Constructors for Document / Element / Text.
Wired into Document arena in Task 3 and taffy trait impl in Task 4.

Claude-Session: https://claude.ai/code/session_01Qo7TYgbGc4JQnghRS6nUJe"
```

---

## Task 3: raikiri-dom document.rs — Document arena + append API

**Files:**
- Create: `crates/raikiri-dom/src/document.rs`

**Interfaces:**
- Consumes: `crate::node::Node`, `smol_str::SmolStr`, `taffy::Style`
- Produces:
  - `pub struct Document` with `pub(crate) nodes: Vec<Node>`, `pub(crate) root: usize`
  - `impl Default for Document`
  - `impl Document { pub fn new(), append_element(...), append_text(...) }`

- [ ] **Step 1: Create `crates/raikiri-dom/src/document.rs`**

```rust
//! Document arena — Vec-backed node arena that implements taffy layout traits
//! (in `taffy_impl.rs`) and raikiri-traits::Dom (in `dom_impl.rs`).

use smol_str::SmolStr;
use taffy::Style;

use crate::node::Node;

/// DOM Document (root + Vec-backed node arena)。
///
/// `nodes` は arena indices を key とする flat storage。index 0 は Document
/// kind の virtual root。HTML の `<html>` element は M1.3 html-parse-basic が
/// index 1 以降に append する想定 (root = 0 の子として)。
pub struct Document {
    pub(crate) nodes: Vec<Node>,
    /// arena index of the Document root (always 0 の予定、明示的に保持して
    /// 将来 detach root 等の変則 case に備える)。
    pub(crate) root: usize,
}

impl Document {
    /// 新しい Document を構築する。arena index 0 に Document kind node を配置。
    pub fn new() -> Self {
        let mut nodes = Vec::with_capacity(16);
        nodes.push(Node::new_document());
        Self { nodes, root: 0 }
    }

    /// Element node を arena に追加する。`parent` が `Some(idx)` の場合
    /// その node の children に append される。`None` の場合 detached (どこにも
    /// 属さない fragment、後で attach する用途)。
    ///
    /// Returns: 追加された node の arena index。
    pub fn append_element(
        &mut self,
        parent: Option<usize>,
        tag: impl Into<SmolStr>,
        style: Style,
    ) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node::new_element(tag.into(), style));
        if let Some(p) = parent {
            self.nodes[p].children.push(id);
        }
        id
    }

    /// Text node を arena に追加する。`parent` の子として append される
    /// (parent は必須、text は必ず attach される)。
    ///
    /// Returns: 追加された node の arena index。
    pub fn append_text(&mut self, parent: usize, text: impl Into<SmolStr>) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node::new_text(text.into()));
        self.nodes[parent].children.push(id);
        id
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 2: Verify compile**

Run: `cargo check -p raikiri-dom` — expected: unchanged (this module not yet declared in lib.rs)。

- [ ] **Step 3: Commit**

```bash
git add crates/raikiri-dom/src/document.rs
git commit -m "feat(raikiri-dom): add Document arena + append API (m1.5)

Document (Vec<Node> + root idx) with append_element / append_text.
Index 0 = Document kind virtual root, HTML root element attaches at
index 1+ by M1.3 html-parse-basic. Taffy trait impls follow in Task 4.

Claude-Session: https://claude.ai/code/session_01Qo7TYgbGc4JQnghRS6nUJe"
```

---

## Task 4: raikiri-dom taffy_impl.rs — 6 taffy traits + unsafe impl Send

**Files:**
- Create: `crates/raikiri-dom/src/taffy_impl.rs`

**Interfaces:**
- Consumes: `crate::document::Document`, taffy traits
- Produces:
  - `impl TraversePartialTree for Document`
  - `impl TraverseTree for Document` (empty marker)
  - `impl CacheTree for Document`
  - `impl LayoutPartialTree for Document`
  - `impl LayoutBlockContainer for Document`
  - `impl LayoutFlexboxContainer for Document`
  - `impl LayoutGridContainer for Document`
  - `unsafe impl Send for Document` with SAFETY doc

- [ ] **Step 1: Create `crates/raikiri-dom/src/taffy_impl.rs`**

```rust
//! Taffy layout trait implementations on Document.
//!
//! spike (`crates/raikiri-feasibility/src/taffy_layout_modes.rs`) の SpikeTree
//! pattern を production 化したもの。実装内容は spike と等価:
//! - `TraversePartialTree`: children iterator
//! - `CacheTree`: per-node cache getter / setter
//! - `LayoutPartialTree`: display に応じて block / flexbox / grid をdispatch
//! - `LayoutBlockContainer / LayoutFlexboxContainer / LayoutGridContainer`:
//!   style getter marker impls

use taffy::{
    compute_block_layout, compute_cached_layout, compute_flexbox_layout, compute_grid_layout,
    compute_leaf_layout, AvailableSpace, CacheTree, Display, Layout, LayoutBlockContainer,
    LayoutFlexboxContainer, LayoutGridContainer, LayoutInput, LayoutOutput, LayoutPartialTree,
    NodeId, Size, Style, TraversePartialTree, TraverseTree,
};

use crate::document::Document;

/// Child iterator for taffy traits.
pub struct ChildIter<'a>(core::slice::Iter<'a, usize>);

impl Iterator for ChildIter<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().copied().map(NodeId::from)
    }
}

impl TraversePartialTree for Document {
    type ChildIter<'a> = ChildIter<'a>;

    fn child_ids(&self, node_id: NodeId) -> Self::ChildIter<'_> {
        ChildIter(self.nodes[usize::from(node_id)].children.iter())
    }

    fn child_count(&self, node_id: NodeId) -> usize {
        self.nodes[usize::from(node_id)].children.len()
    }

    fn get_child_id(&self, node_id: NodeId, index: usize) -> NodeId {
        NodeId::from(self.nodes[usize::from(node_id)].children[index])
    }
}

impl TraverseTree for Document {}

impl CacheTree for Document {
    fn cache_get(&self, node_id: NodeId, inputs: &LayoutInput) -> Option<LayoutOutput> {
        self.nodes[usize::from(node_id)].cache.get(inputs)
    }

    fn cache_store(&mut self, node_id: NodeId, inputs: &LayoutInput, layout_output: LayoutOutput) {
        self.nodes[usize::from(node_id)]
            .cache
            .store(inputs, layout_output)
    }

    fn cache_clear(&mut self, node_id: NodeId) {
        self.nodes[usize::from(node_id)].cache.clear();
    }
}

impl LayoutPartialTree for Document {
    type CustomIdent = String;
    type CoreContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_core_container_style(&self, node_id: NodeId) -> Self::CoreContainerStyle<'_> {
        &self.nodes[usize::from(node_id)].style
    }

    fn set_unrounded_layout(&mut self, node_id: NodeId, layout: &Layout) {
        self.nodes[usize::from(node_id)].unrounded_layout = *layout;
    }

    fn resolve_calc_value(&self, _val: *const (), _basis: f32) -> f32 {
        // M1.5: calc pointer は populate されないため 0.0 を返す (spike と同じ)。
        // M4 で CSS calc() を実装する際に resolver をここに wire する予定。
        0.0
    }

    fn compute_child_layout(&mut self, node_id: NodeId, inputs: LayoutInput) -> LayoutOutput {
        compute_cached_layout(self, node_id, inputs, |tree, node_id, inputs| {
            let idx = usize::from(node_id);
            let is_leaf = tree.nodes[idx].children.is_empty();
            let display = tree.nodes[idx].style.display;
            if is_leaf {
                let style = tree.nodes[idx].style.clone();
                compute_leaf_layout(
                    inputs,
                    &style,
                    |_val, _basis| 0.0,
                    |_known, _avail| Size::ZERO,
                )
            } else {
                match display {
                    Display::Block => compute_block_layout(tree, node_id, inputs, None),
                    Display::Flex => compute_flexbox_layout(tree, node_id, inputs),
                    Display::Grid => compute_grid_layout(tree, node_id, inputs),
                    Display::None => LayoutOutput::HIDDEN,
                }
            }
        })
    }
}

impl LayoutBlockContainer for Document {
    type BlockContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type BlockItemStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_block_container_style(&self, node_id: NodeId) -> Self::BlockContainerStyle<'_> {
        &self.nodes[usize::from(node_id)].style
    }

    fn get_block_child_style(&self, child_node_id: NodeId) -> Self::BlockItemStyle<'_> {
        &self.nodes[usize::from(child_node_id)].style
    }
}

impl LayoutFlexboxContainer for Document {
    type FlexboxContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type FlexboxItemStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_flexbox_container_style(&self, node_id: NodeId) -> Self::FlexboxContainerStyle<'_> {
        &self.nodes[usize::from(node_id)].style
    }

    fn get_flexbox_child_style(&self, child_node_id: NodeId) -> Self::FlexboxItemStyle<'_> {
        &self.nodes[usize::from(child_node_id)].style
    }
}

impl LayoutGridContainer for Document {
    type GridContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type GridItemStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_grid_container_style(&self, node_id: NodeId) -> Self::GridContainerStyle<'_> {
        &self.nodes[usize::from(node_id)].style
    }

    fn get_grid_child_style(&self, child_node_id: NodeId) -> Self::GridItemStyle<'_> {
        &self.nodes[usize::from(child_node_id)].style
    }
}

// SAFETY: taffy の `calc` feature が enable の場合、`Style::Dimension` は
// `CompactLength` 経由で `*const ()` (caller-owned calc expression arena
// pointer) を保持する。Raw pointer は !Send のため `Style: !Send`、そこから
// `Document: !Send` が導出される。
//
// M1.5 は feasibility spike (nzv.6) と同じ approach 1 を採用: Document 内に
// 格納される全 calc pointer は「同じ Document 内 (self-contained arena)」を
// 指す invariant を維持する限り、Document 全体を別 thread へ move しても
// pointer validity は破れない。
//
// M1.5 現段階では calc pointer を populate する経路が存在しない (Node.style
// は Consumer が taffy::Style を直接構築、M1.6 で ComputedValues 変換時も
// `length(px)` / `percent` / `auto` のみ使用予定)。calc pointer が入る余地が
// 生まれるのは M4 sandboxed resolver の CSS calc() 完全 support 段階。
//
// 最終 invariant の確定は m1.17 taffy-layoutbuffer-send-decision に委ねる:
//   - approach 1 (self-contained arena、この unsafe impl のまま)
//   - approach 2 (SendableStyle newtype で pointer を隠蔽)
//   - approach 3 ("no calc across thread boundary" construction guard)
// のいずれかに再整理される。
#[allow(unsafe_code)]
unsafe impl Send for Document {}
```

- [ ] **Step 2: Verify compile**

Run: `cargo check -p raikiri-dom` — expected: unchanged (this module not yet declared).

- [ ] **Step 3: Commit**

```bash
git add crates/raikiri-dom/src/taffy_impl.rs
git commit -m "feat(raikiri-dom): impl taffy 6 layout traits on Document + Send (m1.5)

Promotes SpikeTree pattern (feasibility-report §3.2) to production:
- TraversePartialTree / TraverseTree / CacheTree (arena traversal + cache)
- LayoutPartialTree (display -> block/flex/grid dispatch)
- LayoutBlockContainer / LayoutFlexboxContainer / LayoutGridContainer

unsafe impl Send for Document uses spike's approach 1 (self-contained
arena, M1.5 段階では calc pointer 経路が存在しない)。SAFETY doc で
invariant を明示、m1.17 で最終確定。

Claude-Session: https://claude.ai/code/session_01Qo7TYgbGc4JQnghRS6nUJe"
```

---

## Task 5: raikiri-dom dom_impl.rs — raikiri-traits Dom / Node / Element impl

**Files:**
- Create: `crates/raikiri-dom/src/dom_impl.rs`

**Interfaces:**
- Consumes: `crate::document::Document`, `crate::node::Node`, `raikiri_traits::{Dom, Node, Element, NodeKind, NodeId}`
- Produces:
  - `pub struct NodeRef<'a>` (holds `doc: &'a Document`, `id: usize`)
  - `pub struct ElementRef<'a>` (holds `node: &'a Node`)
  - `pub struct ChildIter<'a>` (wraps slice iter, yields `NodeId`)
  - `impl raikiri_traits::Dom for Document`
  - `impl<'a> raikiri_traits::Node<'a> for NodeRef<'a>`
  - `impl<'a> raikiri_traits::Element<'a> for ElementRef<'a>`

- [ ] **Step 1: Create `crates/raikiri-dom/src/dom_impl.rs`**

```rust
//! `raikiri_traits::Dom / Node / Element` implementations on Document.
//!
//! `NodeRef<'a>` と `ElementRef<'a>` は Document の内部 arena を borrow する
//! 軽量 wrapper。GAT 経由で trait method の返り値型を安定させる。

use raikiri_traits::{Dom as _, Element as _, Node as _, NodeId, NodeKind};

use crate::document::Document;
use crate::node::Node;

/// Node reference borrowed from a Document arena.
pub struct NodeRef<'a> {
    doc: &'a Document,
    id: usize,
}

/// Element reference (kind == Element の Node を型で narrow したもの)。
pub struct ElementRef<'a> {
    node: &'a Node,
}

/// Child NodeId iterator for `Dom::child_ids`.
pub struct ChildIter<'a>(core::slice::Iter<'a, usize>);

impl Iterator for ChildIter<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().copied().map(|i| NodeId::new(i as u64))
    }
}

impl raikiri_traits::Dom for Document {
    type NodeRef<'a> = NodeRef<'a>;
    type ElementRef<'a> = ElementRef<'a>;
    type ChildIter<'a> = ChildIter<'a>;

    fn root_id(&self) -> NodeId {
        NodeId::new(self.root as u64)
    }

    fn node(&self, id: NodeId) -> Option<Self::NodeRef<'_>> {
        let idx = id.0 as usize;
        if idx < self.nodes.len() {
            Some(NodeRef { doc: self, id: idx })
        } else {
            None
        }
    }

    fn child_ids(&self, id: NodeId) -> Self::ChildIter<'_> {
        let idx = id.0 as usize;
        ChildIter(self.nodes[idx].children.iter())
    }
}

impl<'a> raikiri_traits::Node<'a> for NodeRef<'a> {
    type Element<'b>
        = ElementRef<'b>
    where
        Self: 'b;

    fn kind(&self) -> NodeKind {
        self.doc.nodes[self.id].kind
    }

    fn as_element(&self) -> Option<Self::Element<'_>> {
        let node = &self.doc.nodes[self.id];
        matches!(node.kind, NodeKind::Element).then(|| ElementRef { node })
    }

    fn text_content(&self) -> Option<&str> {
        self.doc.nodes[self.id].text_content.as_deref()
    }
}

impl<'a> raikiri_traits::Element<'a> for ElementRef<'a> {
    fn tag_name(&self) -> &str {
        self.node.tag_name.as_deref().unwrap_or("")
    }
}
```

**Note**: `use raikiri_traits::{Dom as _, Element as _, Node as _, ...}` の `as _` は名前衝突を避けるため。Local に `NodeRef` / `ElementRef` を定義しつつ trait methods を呼べる形。

- [ ] **Step 2: Verify compile**

Run: `cargo check -p raikiri-dom` — expected: unchanged (module not yet declared).

- [ ] **Step 3: Commit**

```bash
git add crates/raikiri-dom/src/dom_impl.rs
git commit -m "feat(raikiri-dom): impl raikiri_traits Dom / Node / Element on Document (m1.5)

NodeRef / ElementRef / ChildIter wrappers + trait impls. GAT wired
through so trait methods return concrete NodeRef/ElementRef types (no
boxing, no dyn). raikiri_traits::NodeId (u64 newtype) と taffy::NodeId
との射影は `NodeId::new(idx as u64)` で行う。

Claude-Session: https://claude.ai/code/session_01Qo7TYgbGc4JQnghRS6nUJe"
```

---

## Task 6: raikiri-dom lib.rs — module wiring + re-exports + tests

**Files:**
- Modify: `crates/raikiri-dom/src/lib.rs`

**Interfaces:**
- Consumes: 全 submodule (node, document, taffy_impl, dom_impl)
- Produces: pub mod 宣言 + `pub use` + 4-5 unit tests

- [ ] **Step 1: Replace `crates/raikiri-dom/src/lib.rs` (currently M0 stub)**

現状:
```rust
//! raikiri-dom — DOM data model, layout engine (taffy + parley), and GCPM runtime side.
//!
//! M0 stub. Populated across M1 (dom-model, layout-single-page) through M5
//! (GCPM directive full) and M7 (break policy + probe layout).
```

新しい内容:

```rust
//! raikiri-dom — DOM data model + layout engine (taffy + parley) + GCPM runtime side.
//!
//! M1.5 で node arena + taffy 6 trait impl + raikiri_traits::Dom co-design を
//! 実装。詳細は `docs/superpowers/specs/2026-07-13-raikiri-rebuild-design.md`
//! §4 raikiri-dom 参照。
//!
//! ## Module tour
//!
//! - [`document`] — `Document` arena + `append_element` / `append_text`
//! - [`node`]     — `Node` struct (crate-private)
//! - [`taffy_impl`] — taffy 6 layout trait impls + `unsafe impl Send for Document`
//! - [`dom_impl`]   — `raikiri_traits::Dom / Node / Element` impls + `NodeRef` /
//!                    `ElementRef` types

mod node;

pub mod document;
pub mod dom_impl;
pub mod taffy_impl;

pub use document::Document;
pub use dom_impl::{ChildIter, ElementRef, NodeRef};

#[cfg(test)]
mod tests {
    use super::*;
    use raikiri_traits::{Dom, Element, Node, NodeKind};
    use taffy::prelude::*;
    use taffy::{
        compute_root_layout, AvailableSpace, Dimension, Display, Size, Style,
    };

    fn build_document(display: Display) -> (Document, usize) {
        let mut doc = Document::new();
        let leaf_style = Style {
            size: Size {
                width: Dimension::length(100.0),
                height: Dimension::length(50.0),
            },
            ..Default::default()
        };
        let mut root_style = Style {
            display,
            size: Size {
                width: Dimension::length(400.0),
                height: Dimension::auto(),
            },
            ..Default::default()
        };
        if matches!(display, Display::Grid) {
            root_style.grid_template_columns = vec![length(200.0), length(200.0)];
            root_style.grid_template_rows = vec![length(50.0)];
        }
        // Document node (idx=0) の子として layout root (idx=1) を作る
        let layout_root = doc.append_element(Some(0), "root", root_style);
        doc.append_element(Some(layout_root), "a", leaf_style.clone());
        doc.append_element(Some(layout_root), "b", leaf_style);
        (doc, layout_root)
    }

    #[test]
    fn all_three_display_modes_layout_non_degenerate() {
        for display in [Display::Block, Display::Flex, Display::Grid] {
            let (mut doc, layout_root) = build_document(display);
            compute_root_layout(
                &mut doc,
                taffy::NodeId::from(layout_root),
                Size {
                    width: AvailableSpace::Definite(800.0),
                    height: AvailableSpace::Definite(600.0),
                },
            );
            let layout = doc.nodes[layout_root].unrounded_layout;
            assert!(
                layout.size.width > 0.0 && layout.size.height > 0.0,
                "display={display:?} produced degenerate size ({}x{})",
                layout.size.width,
                layout.size.height,
            );
            assert!(
                (layout.size.width - 400.0).abs() < 0.5,
                "display={display:?}: root width should be ~400, got {}",
                layout.size.width,
            );
        }
    }

    #[test]
    fn independent_documents_lay_out_in_parallel() {
        use std::thread;
        const N: usize = 4;

        let docs: Vec<(Document, usize)> = (0..N)
            .map(|i| {
                let display = match i % 3 {
                    0 => Display::Block,
                    1 => Display::Flex,
                    _ => Display::Grid,
                };
                build_document(display)
            })
            .collect();

        let sizes: Vec<(f32, f32)> = thread::scope(|s| {
            let handles: Vec<_> = docs
                .into_iter()
                .map(|(mut doc, layout_root)| {
                    s.spawn(move || {
                        compute_root_layout(
                            &mut doc,
                            taffy::NodeId::from(layout_root),
                            Size {
                                width: AvailableSpace::Definite(800.0),
                                height: AvailableSpace::Definite(600.0),
                            },
                        );
                        let l = doc.nodes[layout_root].unrounded_layout;
                        (l.size.width, l.size.height)
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });

        assert_eq!(sizes.len(), N);
        for (i, (w, h)) in sizes.iter().enumerate() {
            assert!(
                *w > 0.0 && *h > 0.0,
                "shard {i}: degenerate size ({w}x{h})"
            );
        }
    }

    #[test]
    fn dom_trait_navigation() {
        let mut doc = Document::new();
        let p = doc.append_element(Some(0), "p", Style::default());
        let _t = doc.append_text(p, "hello");

        // root_id は Document kind の virtual root
        let root_id = doc.root_id();
        let root_node = doc.node(root_id).expect("root node exists");
        assert_eq!(root_node.kind(), NodeKind::Document);

        // Document の child = <p> element
        let root_children: Vec<_> = doc.child_ids(root_id).collect();
        assert_eq!(root_children.len(), 1);

        // <p> は Element、tag_name = "p"
        let elem_id = root_children[0];
        let elem_node = doc.node(elem_id).expect("element exists");
        assert_eq!(elem_node.kind(), NodeKind::Element);
        let elem = elem_node.as_element().expect("kind == Element");
        assert_eq!(elem.tag_name(), "p");

        // <p> の child = "hello" text node
        let elem_children: Vec<_> = doc.child_ids(elem_id).collect();
        assert_eq!(elem_children.len(), 1);
        let text_node = doc.node(elem_children[0]).expect("text exists");
        assert_eq!(text_node.kind(), NodeKind::Text);
        assert_eq!(text_node.text_content(), Some("hello"));
        assert!(text_node.as_element().is_none());
    }

    #[test]
    fn document_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Document>();
    }
}
```

- [ ] **Step 2: Verify build + tests + clippy + fmt**

Run: `cargo build -p raikiri-dom` — expected: clean, 0 warnings。
Run: `cargo test -p raikiri-dom` — expected: **4 tests pass**。
Run: `cargo clippy -p raikiri-dom -- -D warnings` — expected: pass (unsafe impl Send は #[allow(unsafe_code)] で局所抑制)。
Run: `cargo fmt -p raikiri-dom -- --check` — expected: clean。

- [ ] **Step 3: Commit**

```bash
git add crates/raikiri-dom/src/lib.rs
git commit -m "feat(raikiri-dom): finalize lib.rs — module wiring + tests (m1.5)

Wires up node / document / taffy_impl / dom_impl modules and re-exports
Document / NodeRef / ElementRef / ChildIter. 4 unit tests:
- all_three_display_modes_layout_non_degenerate (spike promote)
- independent_documents_lay_out_in_parallel (spike promote)
- dom_trait_navigation (Dom / Node / Element trait 経由の tree walk)
- document_is_send (compile-time Send assert for M2 column-count parallel)

Claude-Session: https://claude.ai/code/session_01Qo7TYgbGc4JQnghRS6nUJe"
```

---

## Task 7: Acceptance verification

**Files:**
- None (verification only)

**Interfaces:**
- Consumes: all previous tasks

- [ ] **Step 1: Full-workspace build**

Run: `cargo build --workspace`
Expected: green (M1.5 の raikiri-traits patch + raikiri-dom 変更が他 crate を break していない)。

- [ ] **Step 2: Full-workspace test summary**

Run: `cargo test --workspace 2>&1 | grep "^test result"`
Expected: raikiri-traits 19 pass, raikiri-dom 4 pass, feasibility 18 pass (spike source is still there), 他 0 pass。

- [ ] **Step 3: Strict clippy per package**

Run: `cargo clippy -p raikiri-traits -- -D warnings`
Expected: pass (M1.1 で確立した状態を維持)。

Run: `cargo clippy -p raikiri-dom -- -D warnings`
Expected: pass。unsafe impl Send は #[allow(unsafe_code)] で局所抑制済み。

- [ ] **Step 4: Doc build**

Run: `cargo doc -p raikiri-dom --no-deps`
Expected: clean。

- [ ] **Step 5: Format check**

Run: `cargo fmt -p raikiri-traits -p raikiri-dom -- --check`
Expected: clean。

- [ ] **Step 6: Acceptance checklist**

- [ ] `cargo build -p raikiri-dom` warning 0 で pass
- [ ] `cargo test -p raikiri-dom` — 4 tests pass
- [ ] `cargo clippy -p raikiri-dom -- -D warnings` pass
- [ ] `cargo fmt -p raikiri-dom -- --check` clean
- [ ] `cargo build -p raikiri-traits` warning 0 (patch commit 含む)
- [ ] `cargo test -p raikiri-traits` — 19 tests pass (M1.1 baseline 維持)
- [ ] Document API: `new()`, `default()`, `append_element(parent, tag, style) -> usize`, `append_text(parent, text) -> usize`
- [ ] `impl raikiri_traits::Dom for Document` with `root_id / node / child_ids` methods
- [ ] taffy 6 trait 全部実装: TraversePartialTree, TraverseTree, CacheTree, LayoutPartialTree, LayoutBlockContainer, LayoutFlexboxContainer, LayoutGridContainer
- [ ] `unsafe impl Send for Document {}` に SAFETY doc + m1.17 参照が明示
- [ ] raikiri-traits の Dom / Node / Element trait が GAT + method 付きで populate

- [ ] **Step 7: Git status + branch state**

Run: `git status && git log --oneline main..HEAD`
Expected: working tree clean, 6-7 個の feat/fix commits + 1 plan commit が並ぶ。

---

## Self-Review Checklist

- **Spec coverage**: raikiri-dom §4 の記述 (DOM data model, taffy統合, LayoutBuffer skeleton) をどれだけ M1.5 でカバーするか vs M2 (layoutbuffer-skeleton) / M3 (parley integration) / M4 (GCPM runtime) に defer するか — non-goals として明示済み。
- **Placeholder scan**: TBD / TODO 等の禁止語句なし。placeholder は明示的に "M1.6 で populate", "m1.17 で確定" 等コメント。
- **Type consistency**: raikiri_traits::NodeId (u64 newtype) と taffy::NodeId (u64 wrapper) の射影が Task 5 の ChildIter と Dom impl の両方で `NodeId::new(idx as u64)` で統一。Task 4 の taffy trait ChildIter は taffy::NodeId を返す (異なる型)。名前衝突は module 分離と `as _` import で回避。

---

## Execution Notes

- **branch**: 全 commit は `raikiri-spike-m1.5` branch (`.claude/worktrees/raikiri-spike-m1.5/`)。
- **task granularity**: 6 個の feat/fix commits (Task 1-6) + verification commit なし (Task 7)。TDD 相当は各 task 単位で cargo build + test。
- **CLAUDE.md 準拠**: git push / dolt sync は本 plan 内では行わない。
- **rollback**: 各 task 独立に revert 可能。ただし Task 1 (raikiri-traits patch) は下流全てが依存するので revert 時は連鎖確認要。
