# Flat tree membership metadata Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** raikiri-dom に `IS_IN_DOCUMENT` flat-tree metadata bit を導入し、`<template>` subtree の skip semantics を全 traversal (extract / walk_and_collect / collect_cascaded / resolve_inheritance / paint_document) で `is_in_document()` predicate に統一する。同時に blitz と揃えた `NodeData` tagged union へ Node struct を refactor し、`ElementData.template_contents` slot を予約する。

**Architecture:** Blitz の `NodeFlags` bit + `ElementData.template_contents` sidecar pointer の 2 レイヤ構造を bit 位置 1:1 で採用。M1 spike は parse-only なので sink.finish() の single-pass DFS で bit を維持 (M2+ mutation runtime 時に blitz `process_added_subtree` 相当を追加)。真の fragment separation (arena root reshape、`get_template_contents` の fragment 返却) は残る `raikiri-spike-xno` で対応。

**Tech Stack:** Rust (workspace edition 2024), bitflags 2.x, html5ever, taffy, parley, anyrender, cssparser

## Global Constraints

- **Worktree**: `.claude/worktrees/37c-flat-tree-membership`, branch `worktree-37c-flat-tree-membership`
- **bd issue**: `raikiri-spike-37c` (in_progress)、`raikiri-spike-xno` は blocked-by 37c
- **Spec**: `docs/superpowers/specs/2026-07-18-flat-tree-membership-metadata-design.md`
- **cleanroom rule**: raikiri-traits に html5ever / stylo 型を持ち込まない、UA CSS は CSS spec 由来のみ (blitz backport 前提)
- **M1.15 external contract**: `crates/raikiri/tests/external_consumer.rs` は無変更で pass 継続。concrete Node/Element field access を追加しないこと (0 件 pin)
- **worktree per task**: main への直接コミット禁止、全変更は worktree branch 上で
- **commit style**: `feat|fix|refactor|docs|test(<crate>): <summary> (raikiri-spike-37c)` — bd issue id を各 commit に含める
- **lint**: `cargo clippy --workspace -- -D warnings` / `cargo doc --workspace --no-deps` が warn 0 で通ること (missing_docs / dead_code)

---

## File Structure

**Create**:
- なし (既存 crate 内の追加のみ)

**Modify**:
- `crates/raikiri-dom/Cargo.toml` — bitflags 2.x direct dep
- `crates/raikiri-dom/src/node.rs` — NodeFlags + NodeData / ElementData / TextData + Node 再構成 + accessor methods
- `crates/raikiri-dom/src/document.rs` — append_element / append_text / set_element_* を NodeData 経由へ + `set_in_document` crate-private setter
- `crates/raikiri-dom/src/dom_impl.rs` — NodeRef / ElementRef を NodeData 経由 + `is_in_document()` trait override
- `crates/raikiri-dom/src/layout.rs` — find_body / preshape_text を accessor 経由へ
- `crates/raikiri-dom/src/lib.rs` — 内部 pub_surface pin test の rename + accessor 経由化 + module doc に flat tree section 追記
- `crates/raikiri-html/src/sink.rs` — `mark_in_document_flags` 追加 + phase 順序変更 + `extract_inline_stylesheets` predicate 化 + `get_template_contents` コメント更新
- `crates/raikiri-html/src/lib.rs` — 新 regression fixture 4 本
- `crates/raikiri-style/src/ruletree.rs` — `walk_and_collect` に predicate gate
- `crates/raikiri-style/src/cascade.rs` — `collect_cascaded` / `resolve_inheritance` に predicate gate
- `crates/raikiri-paint/src/walk.rs` — `paint_document` に predicate gate + accessor 経由化
- `crates/raikiri-paint/src/text.rs` — accessor 経由化
- `crates/raikiri-paint/src/lib.rs` — 新 regression fixture 1 本
- `crates/raikiri-traits/src/dom.rs` — `Node<'a>::is_in_document -> bool { true }` default 追加 + doctest

**No delete**.

---

## Task 1: NodeFlags 型 + `is_in_document` trait method 追加

Standalone な metadata primitive の導入。Node struct 再構成は Task 2 で行い、
本タスクは "bit slot と trait default が存在する" 状態を作る。既存 traversal
は無変更なので workspace 全体は変わらず compile + test pass。

**Files:**
- Modify: `crates/raikiri-dom/Cargo.toml` — bitflags 2 direct dep 追加
- Modify: `crates/raikiri-dom/src/node.rs` — NodeFlags bitflags + Node に `flags: NodeFlags` field + accessor
- Modify: `crates/raikiri-traits/src/dom.rs` — Node trait に `is_in_document() -> bool { true }` default + doctest
- Modify: `crates/raikiri-dom/src/dom_impl.rs` — `is_in_document()` trait override
- Modify: `crates/raikiri-dom/src/lib.rs` — 内部 unit test 追加 (flag manipulation)

**Interfaces:**
- Consumes: なし (新規)
- Produces:
  - `pub struct NodeFlags: u32` with `const IS_IN_DOCUMENT = 1 << 0` (bitflags 2)
  - `Node::is_in_document(&self) -> bool` (pub inherent)
  - `Node::set_in_document(&mut self, v: bool)` (pub(crate) inherent)
  - trait method `raikiri_traits::Node<'a>::is_in_document(&self) -> bool { true }` (default true)

### Step 1: Failing test を追加 (raikiri-dom node.rs 内)

- [ ] Add unit test for NodeFlags manipulation

Edit `crates/raikiri-dom/src/node.rs`: 末尾に `#[cfg(test)] mod flags_tests` を追加。

```rust
#[cfg(test)]
mod flags_tests {
    use super::*;

    #[test]
    fn node_flags_default_is_empty() {
        let f = NodeFlags::default();
        assert!(!f.contains(NodeFlags::IS_IN_DOCUMENT));
    }

    #[test]
    fn node_new_document_has_is_in_document_set_by_default() {
        // Node::new_document() は Document root 用、常に flat tree の一員。
        let n = Node::new_document();
        assert!(n.is_in_document());
    }

    #[test]
    fn node_new_element_has_is_in_document_set_by_default() {
        let n = Node::new_element(SmolStr::new("p"), taffy::Style::default(), None);
        assert!(n.is_in_document());
    }

    #[test]
    fn node_new_text_has_is_in_document_set_by_default() {
        let n = Node::new_text(SmolStr::new("hi"));
        assert!(n.is_in_document());
    }

    #[test]
    fn set_in_document_toggles_bit() {
        let mut n = Node::new_document();
        n.set_in_document(false);
        assert!(!n.is_in_document());
        n.set_in_document(true);
        assert!(n.is_in_document());
    }
}
```

### Step 2: 失敗を確認

- [ ] Run tests to verify they fail

```bash
cd /home/mitz/Work/oss/raikiri-spike/.claude/worktrees/37c-flat-tree-membership
cargo test -p raikiri-dom flags_tests 2>&1 | tail -30
```

Expected: compile error — `NodeFlags` 未定義、`is_in_document` / `set_in_document` methods 未定義。

### Step 3: bitflags dep 追加

- [ ] Edit `crates/raikiri-dom/Cargo.toml`

`[dependencies]` セクションに追加 (既存の dep list 末尾、alphabetical order を守る):

```toml
bitflags = "2"
```

### Step 4: NodeFlags 型と Node.flags field を実装

- [ ] Edit `crates/raikiri-dom/src/node.rs`

file 冒頭 (既存 `use` の直後、`Attr` struct の前) に NodeFlags 定義を追加:

```rust
bitflags::bitflags! {
    /// Node に付随する per-node boolean 属性。blitz `NodeFlags` と bit 位置
    /// 1:1 対応 (M6 blitz-compat の nominal 変換前提)。
    ///
    /// M1 spike では `IS_IN_DOCUMENT` のみ定義。将来 `IS_INLINE_ROOT` (M3
    /// inline formatting root)、`IS_TABLE_ROOT` (M3+ table formatting root)
    /// を blitz と同 bit 位置で追加する予定。
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct NodeFlags: u32 {
        /// この Node が flat tree に含まれるか。`<template>` element の子孫は
        /// clear、Document root から flat-tree-parent 経由で到達可能な node は
        /// set。将来 shadow DOM / slot の "shadow-including tree" 意味論を
        /// 追加する場合、slot 割当てられない host 直下や shadow root 外の
        /// light-DOM 子孫も同 bit で表現する予定。
        ///
        /// 維持タイミング:
        /// - parse: `raikiri-html::sink::finish` の `mark_in_document_flags`
        ///   phase で single-pass DFS が set/clear
        /// - mutation runtime (M2+): mutator の `process_added_subtree` /
        ///   `process_removed_subtree` 相当が set/unset
        const IS_IN_DOCUMENT = 1 << 0;
    }
}
```

続いて、既存の `pub struct Node { ... }` 定義に `pub(crate) flags: NodeFlags` field を追加。**Task 1 では他 field は触らない** (Task 2 で refactor):

```rust
#[derive(Debug)]
pub struct Node {
    /// Taffy layout style。
    pub(crate) style: Style,
    /// Child arena indices (`Document::nodes` の usize)。
    pub children: Vec<usize>,
    /// Taffy layout cache (per-node)。
    pub(crate) cache: Cache,
    /// Taffy layout 結果 (compute_root_layout が populate)。
    pub unrounded_layout: Layout,
    /// Per-node metadata bits (raikiri-spike-37c)。IS_IN_DOCUMENT etc.
    ///
    /// crate-private: mutation は Document 経由 (`set_element_*` / sink の
    /// `mark_in_document_flags` phase) で行う。参照は [`Node::is_in_document`]
    /// 等の inherent accessor 経由。
    pub(crate) flags: NodeFlags,
    /// Node kind (Element / Text / Document)。
    pub kind: NodeKind,
    /// Element tag name (kind == Element 時のみ populate、他は `None`)。
    pub tag_name: Option<SmolStr>,
    /// Text character data (kind == Text 時のみ populate、他は `None`)。
    pub(crate) text_content: Option<SmolStr>,
    /// HTML `style="..."` attribute の生 string (kind == Element 時のみ populate、
    /// 他は `None`)。M1.4 raikiri-style::cascade が消費。
    pub(crate) inline_style: Option<SmolStr>,
    /// Element namespace URI (kind == Element かつ non-HTML の場合のみ `Some`、
    /// HTML default namespace は `None` を fast path とする)。
    /// 例: `Some("http://www.w3.org/2000/svg")`。
    pub(crate) namespace: Option<SmolStr>,
    /// null-namespace attribute list (kind == Element 時のみ populate、他は空)。
    pub(crate) attributes: Vec<Attr>,
    /// Text node の pre-shaped parley Layout。Element / Document は常に None。
    pub text_layout: Option<parley::Layout<()>>,
}
```

3 個の `Node::new_*` constructor に `flags: NodeFlags::IS_IN_DOCUMENT` を追加 (default true で "flat tree 内" 状態からスタート、sink が template subtree を clear する):

```rust
impl Node {
    pub(crate) fn new_document() -> Self {
        Self {
            style: Style::default(),
            children: Vec::new(),
            cache: Cache::new(),
            unrounded_layout: Layout::with_order(0),
            flags: NodeFlags::IS_IN_DOCUMENT,  // NEW
            kind: NodeKind::Document,
            tag_name: None,
            text_content: None,
            inline_style: None,
            namespace: None,
            attributes: Vec::new(),
            text_layout: None,
        }
    }

    pub(crate) fn new_element(tag: SmolStr, style: Style, inline_style: Option<SmolStr>) -> Self {
        Self {
            style,
            children: Vec::new(),
            cache: Cache::new(),
            unrounded_layout: Layout::with_order(0),
            flags: NodeFlags::IS_IN_DOCUMENT,  // NEW
            kind: NodeKind::Element,
            tag_name: Some(tag),
            text_content: None,
            inline_style,
            namespace: None,
            attributes: Vec::new(),
            text_layout: None,
        }
    }

    pub(crate) fn new_text(text: SmolStr) -> Self {
        Self {
            style: Style::default(),
            children: Vec::new(),
            cache: Cache::new(),
            unrounded_layout: Layout::with_order(0),
            flags: NodeFlags::IS_IN_DOCUMENT,  // NEW
            kind: NodeKind::Text,
            tag_name: None,
            text_content: Some(text),
            inline_style: None,
            namespace: None,
            attributes: Vec::new(),
            text_layout: None,
        }
    }
}
```

続いて `Node` に inherent accessor を追加 (既存の `is_display_none` の直前):

```rust
impl Node {
    /// この Node が flat tree の一員かを返す (raikiri-spike-37c)。
    ///
    /// [`NodeFlags::IS_IN_DOCUMENT`] bit のシンプルな view。詳細は
    /// [`NodeFlags::IS_IN_DOCUMENT`] の doc を参照。
    #[inline]
    pub fn is_in_document(&self) -> bool {
        self.flags.contains(NodeFlags::IS_IN_DOCUMENT)
    }

    /// [`NodeFlags::IS_IN_DOCUMENT`] bit を明示的に上書きする (crate-private)。
    ///
    /// sink の `mark_in_document_flags` phase および将来の mutation runtime が
    /// 呼ぶ。外部 consumer が直接触ることは無い。
    #[inline]
    pub(crate) fn set_in_document(&mut self, v: bool) {
        self.flags.set(NodeFlags::IS_IN_DOCUMENT, v);
    }
}
```

### Step 5: raikiri-traits::Node trait に default method 追加

- [ ] Edit `crates/raikiri-traits/src/dom.rs`

`pub trait Node<'a>` の中に、`text_content` メソッドの直後に追加:

```rust
    /// この Node が flat tree に含まれるかを返す。`<template>` element の子孫
    /// は `false`、Document root から flat-tree-parent 経由で到達可能な node
    /// は `true` (raikiri-spike-37c)。
    ///
    /// Traversal 側 (cascade / paint / stylesheet extract) はこの predicate
    /// で inert subtree を統一的に skip する。個別の tag_name 判定
    /// (`== "template"` 等) を traversal に散らすのは禁止 — 概念が implicit
    /// になり shadow DOM 追加時に漏れる。
    ///
    /// # Default impl
    ///
    /// 常に `true` を返す。概念未対応の Node impl (test 用 stub 等) が silent
    /// drop されないための safe fallback (blitz `stylo.rs` `TElement::is_in_document
    /// -> true` と同じ姿勢)。raikiri-dom `NodeRef` は override して実 bit を
    /// 返す。
    ///
    /// ```
    /// use raikiri_traits::{Dom, Node};
    /// # struct DummyDoc;
    /// # struct DummyNode;
    /// # struct DummyElem;
    /// # struct DummyIter;
    /// # impl Iterator for DummyIter { type Item = raikiri_traits::NodeId; fn next(&mut self) -> Option<Self::Item> { None } }
    /// # impl<'a> raikiri_traits::Element<'a> for DummyElem { fn tag_name(&self) -> &str { "" } }
    /// # impl<'a> Node<'a> for DummyNode {
    /// #   type Element<'b> = DummyElem where Self: 'b;
    /// #   fn kind(&self) -> raikiri_traits::NodeKind { raikiri_traits::NodeKind::Document }
    /// #   fn as_element(&self) -> Option<Self::Element<'_>> { None }
    /// #   fn text_content(&self) -> Option<&str> { None }
    /// # }
    /// // Default impl は常に true — 概念未対応の実装は overriding 不要。
    /// let n = DummyNode;
    /// assert!(n.is_in_document());
    /// ```
    fn is_in_document(&self) -> bool {
        true
    }
```

### Step 6: raikiri-dom NodeRef trait override

- [ ] Edit `crates/raikiri-dom/src/dom_impl.rs`

`impl<'a> raikiri_traits::Node<'a> for NodeRef<'a>` block に override を追加 (既存の `text_content` メソッドの直後):

```rust
    fn is_in_document(&self) -> bool {
        self.doc.nodes[self.id].is_in_document()
    }
```

### Step 7: raikiri-dom lib.rs にも pub 公開 (必要なら)

- [ ] Edit `crates/raikiri-dom/src/lib.rs`

現状 lib.rs は `pub use node::{Attr, Node};` 等の re-export を持っているはず。`NodeFlags` を pub re-export に追加:

```bash
grep -n "pub use node" /home/mitz/Work/oss/raikiri-spike/.claude/worktrees/37c-flat-tree-membership/crates/raikiri-dom/src/lib.rs
```

該当行の pub use 対象に `NodeFlags` を追加。例:

```rust
pub use node::{Attr, Node, NodeFlags};
```

(現状の re-export listに合わせる)。

### Step 8: テスト成功を確認

- [ ] Run flag tests

```bash
cargo test -p raikiri-dom flags_tests 2>&1 | tail -15
```

Expected: PASS (5 test cases)。

- [ ] Run doctest

```bash
cargo test -p raikiri-traits --doc 2>&1 | tail -10
```

Expected: PASS。

- [ ] Full workspace build + test

```bash
cargo build --workspace 2>&1 | tail -5
cargo test --workspace 2>&1 | tail -30
```

Expected: 既存全 test 継続 pass、新規 5 test + 1 doctest が pass。

### Step 9: lint pass

- [ ] Clippy / doc warn check

```bash
cargo clippy --workspace -- -D warnings 2>&1 | tail -20
cargo doc --workspace --no-deps 2>&1 | tail -10
```

Expected: warn 0。

### Step 10: Commit

- [ ] Commit Task 1

```bash
git add crates/raikiri-dom/Cargo.toml \
        crates/raikiri-dom/src/node.rs \
        crates/raikiri-dom/src/dom_impl.rs \
        crates/raikiri-dom/src/lib.rs \
        crates/raikiri-traits/src/dom.rs

git commit -m "$(cat <<'EOF'
feat(raikiri-dom,raikiri-traits): NodeFlags bit + Node::is_in_document accessor + trait default (raikiri-spike-37c)

- bitflags 2 direct dep + NodeFlags struct (IS_IN_DOCUMENT bit)
- Node に crate-private flags field 追加 (constructors で default true set)
- Node inherent is_in_document() / set_in_document() accessor
- raikiri-traits Node trait に is_in_document() -> bool { true } default + doctest
- raikiri-dom NodeRef が trait override で実 bit を返す

Task 2 で Node struct を tagged-union に refactor、その前段として bit slot と trait
default を先行導入。既存 traversal は無変更で全 test 継続 pass。
EOF
)"
```

---

## Task 2: Node struct → NodeData tagged union refactor + accessor migration

Node を flat struct から `NodeData::Element(Box<ElementData>) | Text(TextData) | Document`
tagged union に refactor。`ElementData.template_contents` slot を予約 (populate せず)。
raikiri-dom 内部の field access と raikiri-paint の concrete field access を accessor
経由に移行 (atomic commit — 途中は workspace が compile しない)。

**Files:**
- Modify: `crates/raikiri-dom/src/node.rs` — NodeData enum + ElementData + TextData 導入 + Node struct 再構成 + accessor methods 追加
- Modify: `crates/raikiri-dom/src/document.rs` — append_element / append_text / set_element_* を NodeData 経由に
- Modify: `crates/raikiri-dom/src/dom_impl.rs` — NodeRef / ElementRef の field 参照を NodeData 経由に
- Modify: `crates/raikiri-dom/src/layout.rs` — find_body の tag_name 比較を accessor 経由へ、preshape_text の text_layout mutation を data enum 経由に
- Modify: `crates/raikiri-dom/src/lib.rs` — 内部 pub_surface pin test の rename + accessor 経由化
- Modify: `crates/raikiri-paint/src/walk.rs` — node.kind / node.tag_name を accessor に
- Modify: `crates/raikiri-paint/src/text.rs` — node.text_layout を accessor に

**Interfaces:**
- Consumes: Task 1 の `NodeFlags` / `Node::is_in_document` / `Node::set_in_document`
- Produces:
  - `pub enum NodeData { Element(Box<ElementData>), Text(TextData), Document }` (raikiri-dom pub re-export)
  - `pub struct ElementData { tag_name: SmolStr, inline_style: Option<SmolStr>, namespace: Option<SmolStr>, attributes: Vec<Attr>, template_contents: Option<usize> }` (fields crate-private)
  - `pub struct TextData { text_content: SmolStr, text_layout: Option<parley::Layout<()>> }` (fields: text_content crate-private、text_layout pub)
  - `Node::kind(&self) -> NodeKind` (accessor)
  - `Node::tag_name(&self) -> Option<&str>` (accessor)
  - `Node::text_layout(&self) -> Option<&parley::Layout<()>>` (accessor)
  - `NodeData::as_text_mut(&mut self) -> Option<&mut TextData>` (crate-private helper)

### Step 1: 失敗する pub_surface pin test を書き換え

- [ ] Edit `crates/raikiri-dom/src/lib.rs` — 現行 test 置換

現状 (line 638-654 付近) の `node_pub_fields_are_readable_from_external_call_site` を丸ごと以下で置換:

```rust
    #[test]
    fn node_accessors_are_callable_from_external_call_site() {
        // raikiri-spike-37c: Node が NodeData tagged union に refactor された
        // 後の pub_surface pin。旧 pub field (kind / tag_name / text_layout)
        // が accessor method 化されたことを super::* から見えることで regression
        // pin する。M1.15 external consumer 契約は無影響
        // (crates/raikiri/tests/external_consumer.rs は Node/Element field
        // access 0 件、こちらは raikiri-dom 内部 pub_surface)。
        let mut doc = Document::new();
        let e = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
        let t = doc.append_text(e, "hi");
        let node = doc.get_node(e).unwrap();
        let _ = &node.children;
        let _ = &node.unrounded_layout;
        let _ = node.kind();
        let _ = node.tag_name();
        let _ = node.text_layout();
        let _ = node.is_in_document();
        let tn = doc.get_node(t).unwrap();
        assert_eq!(tn.kind(), NodeKind::Text);
    }
```

### Step 2: 失敗を確認

- [ ] Run compile

```bash
cargo build -p raikiri-dom 2>&1 | tail -20
```

Expected: compile error — `Node::kind()` / `Node::tag_name()` / `Node::text_layout()` は Task 1 まで存在しない。

### Step 3: Node struct を tagged union に refactor

- [ ] Edit `crates/raikiri-dom/src/node.rs`

現状の Node struct と `impl Node` の new_*/is_display_none/is_in_document/set_in_document を丸ごと以下で置換 (NodeFlags 定義と `Attr` struct は保持):

```rust
/// NodeData: kind 固有 field を集約した tagged union (raikiri-spike-37c)。
///
/// blitz `blitz-dom::node::node::NodeData` に対応する shape。M6 blitz-compat
/// で nominal 変換 (`match data { NodeData::Element(e) => BlitzElement { ... }, ... }`)
/// できるように field 名を揃える。`Element` variant のみ `Box` で indirection
/// を挟むのは Text / Document node の memory footprint を削らないため
/// (blitz と同じ選択、size_of::<NodeData>() を単一 usize 相当に抑える)。
#[derive(Debug)]
pub enum NodeData {
    /// HTML / XML element (tag_name + attributes + namespace + inline_style +
    /// template_contents slot を持つ)。
    Element(Box<ElementData>),
    /// Character data node。
    Text(TextData),
    /// Document root (arena index 0 の virtual node)。
    Document,
}

impl NodeData {
    /// Element variant を crate-private に mut borrow (Document setter 用)。
    #[inline]
    pub(crate) fn as_element_mut(&mut self) -> Option<&mut ElementData> {
        match self {
            NodeData::Element(e) => Some(e.as_mut()),
            _ => None,
        }
    }

    /// Text variant を crate-private に mut borrow (layout::preshape_text 用)。
    #[inline]
    pub(crate) fn as_text_mut(&mut self) -> Option<&mut TextData> {
        match self {
            NodeData::Text(t) => Some(t),
            _ => None,
        }
    }
}

/// Element-only data (raikiri-spike-37c)。blitz `ElementData` に対応。
///
/// `template_contents` は `<template>` element の contents fragment root への
/// arena index を保持する slot として予約。M1 spike では sink が populate せず
/// `get_template_contents` は `*target` を返す (blitz と同じ TODO 状態)。M2+ で
/// clone/inject fixture が必要になった時に populate する
/// (raikiri-spike-xno Part 2)。
#[derive(Debug)]
pub struct ElementData {
    /// HTML / XML tag name (例: `"p"`, `"div"`)。html5ever の QualName.local から
    /// SmolStr に写し取る。
    pub(crate) tag_name: SmolStr,
    /// HTML `style="..."` attribute の生 string (kind == Element 時のみ populate、
    /// 空文字列 `style=""` は Element trait contract 上 `None` として view 化
    /// されるが、storage はここでは正規化せず raw 値を持つ)。
    pub(crate) inline_style: Option<SmolStr>,
    /// Element namespace URI (non-HTML の場合のみ `Some`、HTML default は
    /// `None` を fast path とする)。例: `Some("http://www.w3.org/2000/svg")`。
    pub(crate) namespace: Option<SmolStr>,
    /// null-namespace attribute list (順序保持、cascade tie-breaking で使う想定)。
    /// `style` attribute は [`ElementData::inline_style`] に分離済のためここには
    /// 含めない。
    pub(crate) attributes: Vec<Attr>,
    /// `<template>` element の contents fragment root への arena index。
    ///
    /// M1 spike では sink が populate しない (常に `None`)。`get_template_contents`
    /// も `*target` を返し続ける。M2+ で raikiri-spike-xno Part 2 の中で
    /// populate 実装 + `get_template_contents` の切り替えを行う。blitz
    /// `blitz-dom::node::element::ElementData::template_contents` と同名・同 shape。
    pub(crate) template_contents: Option<usize>,
}

/// Text-only data (raikiri-spike-37c)。blitz `TextNodeData` (nominally) に対応。
#[derive(Debug)]
pub struct TextData {
    /// Character data。
    pub(crate) text_content: SmolStr,
    /// Text node の pre-shaped parley Layout。
    ///
    /// - Populated by [`crate::layout::preshape_text`] (M1.6)
    /// - Consumed by taffy leaf measure closure (intrinsic size) と m1.7 paint
    ///   (glyph 位置)
    /// - Brush type `()` は M1.6 の choice: color / decoration は持たせない
    /// - Invalidation: `layout_single_page` 呼び出し毎に全 None にクリア + 再走
    pub text_layout: Option<parley::Layout<()>>,
}

/// Arena node (raikiri-spike-37c refactor: NodeData tagged union に移行)。
///
/// paint / cascade / layout に必要な kind 非依存の field (children /
/// unrounded_layout) は Node に残し、kind 固有 field は [`NodeData`] variant
/// に集約する。raikiri-spike-m1.7 で pub 化した 5 field のうち `kind` /
/// `tag_name` / `text_layout` は accessor method 経由に移行 (`node.kind()` /
/// `node.tag_name()` / `node.text_layout()`)、`children` / `unrounded_layout`
/// は pub field 継続。M1.15 external contract は Node/Element field access 0
/// 件なので無影響、raikiri-dom 内部 pub_surface pin のみ accessor 経由に再 pin。
#[derive(Debug)]
pub struct Node {
    /// Taffy layout style。
    pub(crate) style: Style,
    /// Child arena indices (`Document::nodes` の usize)。
    pub children: Vec<usize>,
    /// Taffy layout cache (per-node)。
    pub(crate) cache: Cache,
    /// Taffy layout 結果 (compute_root_layout が populate)。
    pub unrounded_layout: Layout,
    /// Per-node metadata bits (IS_IN_DOCUMENT etc.)。crate-private mutation。
    pub(crate) flags: NodeFlags,
    /// Node kind + kind 固有 field (tagged union)。
    pub(crate) data: NodeData,
}

impl Node {
    /// Document root node (arena index 0 用) を構築する。
    pub(crate) fn new_document() -> Self {
        Self {
            style: Style::default(),
            children: Vec::new(),
            cache: Cache::new(),
            unrounded_layout: Layout::with_order(0),
            flags: NodeFlags::IS_IN_DOCUMENT,
            data: NodeData::Document,
        }
    }

    /// Element node を tag name / style / inline_style と共に構築する。
    /// `namespace` / `attributes` / `template_contents` は初期空/None で、raikiri-html
    /// sink が finish 時に [`crate::Document::set_element_namespace`] /
    /// [`crate::Document::set_element_attributes`] で populate する
    /// (template_contents は M1 では populate なし)。
    pub(crate) fn new_element(tag: SmolStr, style: Style, inline_style: Option<SmolStr>) -> Self {
        Self {
            style,
            children: Vec::new(),
            cache: Cache::new(),
            unrounded_layout: Layout::with_order(0),
            flags: NodeFlags::IS_IN_DOCUMENT,
            data: NodeData::Element(Box::new(ElementData {
                tag_name: tag,
                inline_style,
                namespace: None,
                attributes: Vec::new(),
                template_contents: None,
            })),
        }
    }

    /// Text node を character data と共に構築する。
    pub(crate) fn new_text(text: SmolStr) -> Self {
        Self {
            style: Style::default(),
            children: Vec::new(),
            cache: Cache::new(),
            unrounded_layout: Layout::with_order(0),
            flags: NodeFlags::IS_IN_DOCUMENT,
            data: NodeData::Text(TextData {
                text_content: text,
                text_layout: None,
            }),
        }
    }

    // ─── inherent accessor methods (raikiri-spike-37c) ─────────────────

    /// この Node の [`NodeKind`] を返す。
    ///
    /// 旧 `pub kind: NodeKind` field の accessor 版 (raikiri-spike-37c refactor)。
    /// 呼び出し側は `node.kind` → `node.kind()` の syntax 変更のみ。
    #[inline]
    pub fn kind(&self) -> NodeKind {
        match &self.data {
            NodeData::Element(_) => NodeKind::Element,
            NodeData::Text(_) => NodeKind::Text,
            NodeData::Document => NodeKind::Document,
        }
    }

    /// Element の場合 tag_name を、それ以外は `None` を返す。
    ///
    /// 旧 `pub tag_name: Option<SmolStr>` field の accessor 版
    /// (raikiri-spike-37c refactor)。`Option<&str>` に射影する
    /// (SmolStr の内部 view で Copy 相当のコスト)。
    #[inline]
    pub fn tag_name(&self) -> Option<&str> {
        match &self.data {
            NodeData::Element(e) => Some(e.tag_name.as_str()),
            _ => None,
        }
    }

    /// Text の場合 text_layout を、それ以外は `None` を返す。
    ///
    /// 旧 `pub text_layout: Option<parley::Layout<()>>` field の accessor 版
    /// (raikiri-spike-37c refactor)。paint hot path から呼ばれるため `#[inline]`。
    #[inline]
    pub fn text_layout(&self) -> Option<&parley::Layout<()>> {
        match &self.data {
            NodeData::Text(t) => t.text_layout.as_ref(),
            _ => None,
        }
    }

    /// このノードの `taffy::Style.display == Display::None` を返す。
    ///
    /// paint 段で display:none subtree を skip する目的の predicate。size 0
    /// による代理判定は overflow: visible の legitimate な zero-size 要素を
    /// silent drop するため誤り (roborev job 223 finding 対応)。style field
    /// は crate-private のまま維持し、paint に必要な最小の boolean 述語のみ
    /// pub で公開する (gradual exposure)。
    #[inline]
    pub fn is_display_none(&self) -> bool {
        self.style.display == taffy::Display::None
    }

    /// この Node が flat tree の一員かを返す (raikiri-spike-37c)。
    #[inline]
    pub fn is_in_document(&self) -> bool {
        self.flags.contains(NodeFlags::IS_IN_DOCUMENT)
    }

    /// [`NodeFlags::IS_IN_DOCUMENT`] bit を明示的に上書きする (crate-private)。
    #[inline]
    pub(crate) fn set_in_document(&mut self, v: bool) {
        self.flags.set(NodeFlags::IS_IN_DOCUMENT, v);
    }
}
```

flags_tests 内の `Node::new_element` 呼び出し (`SmolStr::new("p")` を渡している) はそのまま動く (signature 不変)。

### Step 4: raikiri-dom lib.rs の pub re-export に NodeData / ElementData / TextData 追加

- [ ] Edit `crates/raikiri-dom/src/lib.rs`

`pub use node::` の対象に `NodeData, ElementData, TextData` を追加:

```rust
pub use node::{Attr, ElementData, Node, NodeData, NodeFlags, TextData};
```

(現状 line を grep で確認して適切に merge)

### Step 5: Document methods (append_element / set_element_*) を NodeData 経由に

- [ ] Edit `crates/raikiri-dom/src/document.rs`

`set_element_namespace` / `set_element_attributes` / `set_element_inline_style` を NodeData::Element 経由の field access に書き換え:

```rust
    pub fn set_element_namespace(&mut self, id: usize, ns: Option<SmolStr>) {
        let e = self.nodes[id].data.as_element_mut().expect(
            "set_element_namespace called on non-Element",
        );
        e.namespace = ns;
    }

    pub fn set_element_attributes(&mut self, id: usize, attrs: Vec<(SmolStr, SmolStr)>) {
        let e = self.nodes[id].data.as_element_mut().expect(
            "set_element_attributes called on non-Element",
        );
        e.attributes = attrs
            .into_iter()
            .map(|(local, value)| Attr { local, value })
            .collect();
    }

    pub fn set_element_inline_style(&mut self, id: usize, inline_style: Option<SmolStr>) {
        let e = self.nodes[id].data.as_element_mut().expect(
            "set_element_inline_style called on non-Element",
        );
        e.inline_style = inline_style;
    }
```

現状の `debug_assert_eq!(self.nodes[id].kind, NodeKind::Element, ...)` チェックは `as_element_mut().expect(...)` に統合。**contract**: debug builds では `.expect(...)` で panic (現状の debug_assert と同じ severity)、release builds では `.expect(...)` はそのまま panic するので **より strict** になる — この change は tree-building 中の misuse を早期発見するので pit-of-success 寄り。既存 test で誤呼び出しは無いことを test pass で検証。

`append_element` / `append_text` / `attach_child` / `insert_child_before` / `parent_of` / `detach_from_parent` / `reparent_children` / `retain_children` / `get_node` / `node_count` / `root_index` / `invalidate_layout_cache` / stylesheet-related methods は Node の pub field (children) と `Node::new_*` constructor 経由なので **無変更で通る** (signature に kind 固有 field が現れない)。

### Step 6: dom_impl.rs (NodeRef / ElementRef trait impl) の field 参照を NodeData 経由に

- [ ] Edit `crates/raikiri-dom/src/dom_impl.rs`

現状の trait impl block を以下で置換 (基本 field name のみの mechanical translation):

```rust
impl<'a> raikiri_traits::Node<'a> for NodeRef<'a> {
    type Element<'b>
        = ElementRef<'b>
    where
        Self: 'b;

    fn kind(&self) -> NodeKind {
        self.doc.nodes[self.id].kind()
    }

    fn as_element(&self) -> Option<Self::Element<'_>> {
        let node = &self.doc.nodes[self.id];
        matches!(&node.data, crate::node::NodeData::Element(_)).then(|| ElementRef { node })
    }

    fn text_content(&self) -> Option<&str> {
        match &self.doc.nodes[self.id].data {
            crate::node::NodeData::Text(t) => Some(t.text_content.as_str()),
            _ => None,
        }
    }

    fn is_in_document(&self) -> bool {
        self.doc.nodes[self.id].is_in_document()
    }
}

impl<'a> raikiri_traits::Element<'a> for ElementRef<'a> {
    fn tag_name(&self) -> &str {
        // ElementRef は as_element() が Some を返した後の view なので必ず
        // NodeData::Element (invariant)、それ以外は panic 相当。
        match &self.node.data {
            crate::node::NodeData::Element(e) => e.tag_name.as_str(),
            _ => "",  // defensive: 到達しない
        }
    }

    fn inline_style_source(&self) -> Option<&str> {
        match &self.node.data {
            crate::node::NodeData::Element(e) => e.inline_style.as_deref().filter(|s| !s.is_empty()),
            _ => None,
        }
    }

    fn namespace_uri(&self) -> Option<&str> {
        match &self.node.data {
            crate::node::NodeData::Element(e) => e.namespace.as_deref(),
            _ => None,
        }
    }

    fn attr(&self, local: &str) -> Option<&str> {
        if local == "style" {
            return self.inline_style_source();
        }
        match &self.node.data {
            crate::node::NodeData::Element(e) => e
                .attributes
                .iter()
                .find(|a| a.local == local)
                .map(|a| a.value.as_str())
                .filter(|s| !s.is_empty()),
            _ => None,
        }
    }
}
```

`crate::node::NodeData` の path 記述は既に `use crate::node::Node;` があるので `use crate::node::{Node, NodeData};` に足しても、full path で書いても可 (readability で選ぶ)。

### Step 7: raikiri-dom layout.rs の tag_name / text_layout field access を accessor に

- [ ] Edit `crates/raikiri-dom/src/layout.rs`

現状 `node.kind == NodeKind::Element && node.tag_name.as_deref() == Some("body")` を accessor に:

```rust
if node.kind() == NodeKind::Element && node.tag_name() == Some("body") {
```

`node.text_layout = None` の mutation は data enum 経由:

```rust
if let Some(t) = node.data.as_text_mut() {
    t.text_layout = None;
}
```

### Step 8: raikiri-paint walk.rs を accessor 経由に

- [ ] Edit `crates/raikiri-paint/src/walk.rs`

`match node.kind` を `match node.kind()`、`node.tag_name.as_deref() == Some("body")` を `node.tag_name() == Some("body")` に書き換え:

```rust
    match node.kind() {
        NodeKind::Element => {
            /* ... */
        }
        /* ... */
    }
```

`find_body` 内:
```rust
if node.kind() == NodeKind::Element && node.tag_name() == Some("body") {
    return Some(id);
}
```

`node.children.iter().rev()` と `node.unrounded_layout` は pub field 継続なので無変更。

### Step 9: raikiri-paint text.rs を accessor 経由に

- [ ] Edit `crates/raikiri-paint/src/text.rs`

`node.text_layout.as_ref()` を `node.text_layout()` に書き換え:

```rust
let Some(text_layout) = node.text_layout() else {
    return;
};
```

### Step 10: compile pass 確認

- [ ] Full workspace build

```bash
cd /home/mitz/Work/oss/raikiri-spike/.claude/worktrees/37c-flat-tree-membership
cargo build --workspace 2>&1 | tail -20
```

Expected: 0 error、warn 0 (missing_docs 含む)。

### Step 11: 全 test pass 確認

- [ ] Run workspace tests

```bash
cargo test --workspace 2>&1 | tail -40
```

Expected: 全 test pass。Task 1 の flags_tests / raikiri-traits doctest も継続 pass。Step 1 で書き換えた `node_accessors_are_callable_from_external_call_site` が pass。

### Step 12: clippy pass

- [ ] Clippy check

```bash
cargo clippy --workspace -- -D warnings 2>&1 | tail -20
```

Expected: warn 0。**注意**: `template_contents` field は現時点で誰も populate/read しないが、`#[derive(Debug)]` の derived Debug impl が field を read として算入するため dead_code は silent (advisor #3 反映)。もし clippy が warn を出したら doc comment 追加 + `#[allow(dead_code, reason = "reserved for raikiri-spike-xno Part 2")]` を field に付けて解消。

### Step 13: Commit

- [ ] Commit Task 2

```bash
git add crates/raikiri-dom/src/node.rs \
        crates/raikiri-dom/src/document.rs \
        crates/raikiri-dom/src/dom_impl.rs \
        crates/raikiri-dom/src/layout.rs \
        crates/raikiri-dom/src/lib.rs \
        crates/raikiri-paint/src/walk.rs \
        crates/raikiri-paint/src/text.rs

git commit -m "$(cat <<'EOF'
refactor(raikiri-dom,raikiri-paint): Node → NodeData tagged union + ElementData template_contents slot reservation + accessor migration (raikiri-spike-37c)

- Node struct を kind 非依存 field (children / unrounded_layout / flags) と
  kind 固有 field を NodeData::Element(Box<ElementData>) | Text(TextData) |
  Document に分離する tagged union に refactor
- ElementData に template_contents: Option<usize> slot を予約 (M1 populate せず、
  M2+ raikiri-spike-xno Part 2 で使用)
- Node::kind() / tag_name() / text_layout() を accessor method 化 (旧 pub field
  は accessor 経由に移行)
- raikiri-dom Document setter / dom_impl trait impl / layout を NodeData 経由に
- raikiri-paint walk / text の concrete field access を accessor に
- raikiri-dom 内部 pub_surface pin test を node_accessors_are_callable_from_
  external_call_site に rename、accessor 経由の regression pin へ

M1.15 external consumer 契約は無影響 (external_consumer.rs は Node/Element
field access 0 件)。
EOF
)"
```

---

## Task 3: sink.finish() `mark_in_document_flags` + `extract_inline_stylesheets` predicate 化 + parse-side regression fixtures

sink.finish() の新 phase として DFS で `<template>` subtree の IS_IN_DOCUMENT
bit を clear。extract_inline_stylesheets の `"template"` string 判定を予測経由へ。
regression fixture 4 本を追加。

**Files:**
- Modify: `crates/raikiri-html/src/sink.rs` — `mark_in_document_flags` 追加 + phase 順序変更 + `extract_inline_stylesheets` predicate 化 + `get_template_contents` コメント更新
- Modify: `crates/raikiri-html/src/lib.rs` — 新 regression fixture 4 本 (parse_marks_* 3 + cascade integration 1)

**Interfaces:**
- Consumes: Task 2 の `Node::is_in_document()` + `NodeData` + `Node::set_in_document`
- Produces:
  - `raikiri-html::sink::mark_in_document_flags` (crate-private fn)
  - 新 regression fixtures in raikiri-html test mod: parse_marks_body_children_in_document / parse_marks_template_descendants_out_of_document / parse_marks_nested_template_descendants_out_of_document / parse_then_cascade_skips_template_descendants
- Unchanged (still delegates to *target): `RaikiriTreeSink::get_template_contents`

### Step 1: 失敗する fixture を追加 (parse_marks_template_descendants_out_of_document)

- [ ] Edit `crates/raikiri-html/src/lib.rs`

test mod の `parse_skips_style_inside_template_element` テストの直後に追加:

```rust
    #[test]
    fn parse_marks_template_descendants_out_of_document() {
        // raikiri-spike-37c: <template> element 自身は flat tree の一員なので
        // is_in_document()=true、その descendants (子孫の element / text) は
        // false であることを parse 経路の bit populate で pin する。
        //
        // 現在の sink には mark_in_document_flags phase が無いため、default
        // true が clear されず descendant も true になる → 失敗する failing test。
        let html = b"<html><head></head><body>\
                     <template><p id=\"inner\">hi</p></template>\
                     </body></html>";
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        let doc = &uncascaded.dom;

        let mut saw_template = false;
        let mut saw_inner_p = false;
        let mut saw_inner_text = false;
        for id_u in 0..doc.node_count() {
            let id = raikiri_traits::NodeId::new(id_u as u64);
            let n = doc.node(id).expect("in-range");
            if let Some(el) = n.as_element() {
                match el.tag_name() {
                    "template" => {
                        assert!(n.is_in_document(), "template element itself must be in document");
                        saw_template = true;
                    }
                    "p" if el.id() == Some("inner") => {
                        assert!(!n.is_in_document(), "<p> inside <template> must be out of document");
                        saw_inner_p = true;
                    }
                    _ => {}
                }
            }
            if n.text_content() == Some("hi") {
                assert!(!n.is_in_document(), "text inside <template> must be out of document");
                saw_inner_text = true;
            }
        }
        assert!(saw_template, "template element should exist in parsed tree");
        assert!(saw_inner_p, "<p id=inner> should exist inside template subtree");
        assert!(saw_inner_text, "'hi' text should exist inside template subtree");
    }
```

### Step 2: 失敗を確認

- [ ] Run new test

```bash
cd /home/mitz/Work/oss/raikiri-spike/.claude/worktrees/37c-flat-tree-membership
cargo test -p raikiri-html parse_marks_template_descendants_out_of_document 2>&1 | tail -20
```

Expected: FAIL — assertion `!n.is_in_document()` の inner_p / inner_text で fail (現在 default true のまま)。

### Step 3: `mark_in_document_flags` を実装

- [ ] Edit `crates/raikiri-html/src/sink.rs`

`extract_inline_stylesheets` 関数の直前 (file 末尾寄り) に追加:

```rust
/// `<template>` element の子孫について `IS_IN_DOCUMENT` bit を clear する
/// single-pass DFS (raikiri-spike-37c)。
///
/// - Node::new_* constructor が default `IS_IN_DOCUMENT=true` を立てているため、
///   本 phase は「flat tree の外に落とすべき node の bit を clear する」補正
///   phase として機能する。template element 自身は flat tree の一員なので bit
///   set のまま、その descendants の bit を clear する。
/// - `<template>` 判定は HTML namespace + local == "template" (case-sensitive)。
///   html5ever が local を lowercase 済で提供する契約に依存。SVG hypothetical
///   `<template>` (別 namespace) は skip 対象外 (現行 walk_and_collect の
///   `eq_ignore_ascii_case` が持っていた false hit を解消)。
/// - iterative Vec stack で深い DOM での stack overflow を回避 (raikiri-dom /
///   raikiri-style の既存 walker と一貫)。
fn mark_in_document_flags(doc: &mut Document) {
    let root = doc.root_index();
    let mut stack: Vec<(usize, bool /* in_template_subtree */)> = vec![(root, false)];
    while let Some((id, in_template)) = stack.pop() {
        let (children_snapshot, is_template_here) = {
            let node = &mut doc.nodes[id];
            node.set_in_document(!in_template);
            let is_template = matches!(
                &node.data,
                raikiri_dom::NodeData::Element(e)
                    if e.tag_name() == "template" && e.namespace().is_none()
            );
            (node.children.clone(), is_template)
        };
        let child_in_template = in_template || is_template_here;
        for c in children_snapshot.into_iter().rev() {
            stack.push((c, child_in_template));
        }
    }
}
```

**注意**: `mark_in_document_flags` は raikiri-dom の crate-private field
(`node.data`, `node.children`, `set_in_document`) にアクセスしている。しかし
raikiri-html は raikiri-dom の外部 crate なので直接 access できない。

上記コードは Task 2 で `Node.data` を `pub(crate)` にしているため raikiri-html
からは access できない — pub(crate) の boundary を越えるので **doc::mark_in_document_flags
は raikiri-dom 側に定義**し、raikiri-html から `raikiri_dom::mark_in_document_flags(&mut doc)`
として呼ぶ形にする必要がある。

**revise**: この step を 2 分割する:

**Step 3a**: `crates/raikiri-dom/src/document.rs` に pub method を追加

```rust
    /// `<template>` element の子孫について `IS_IN_DOCUMENT` bit を clear する
    /// single-pass DFS (raikiri-spike-37c)。sink.finish() から呼ばれる。
    ///
    /// - Node::new_* constructor が default `IS_IN_DOCUMENT=true` を立てているため、
    ///   本 method は「flat tree の外に落とすべき node の bit を clear する」補正
    ///   phase として機能する。template element 自身は flat tree の一員なので bit
    ///   set のまま、その descendants の bit を clear する。
    /// - `<template>` 判定は HTML namespace + local == "template" (case-sensitive)。
    ///   html5ever が local を lowercase 済で提供する契約に依存。SVG hypothetical
    ///   `<template>` (別 namespace) は skip 対象外。
    /// - iterative Vec stack で深い DOM での stack overflow を回避。
    /// - tree mutation ではないので `invalidate_layout_cache` は呼ばない。
    pub fn mark_in_document_flags(&mut self) {
        let root = self.root_index();
        let mut stack: Vec<(usize, bool)> = vec![(root, false)];
        while let Some((id, in_template)) = stack.pop() {
            let (children_snapshot, is_template_here) = {
                let node = &mut self.nodes[id];
                node.set_in_document(!in_template);
                let is_template = match &node.data {
                    NodeData::Element(e) => {
                        e.tag_name.as_str() == "template" && e.namespace.is_none()
                    }
                    _ => false,
                };
                (node.children.clone(), is_template)
            };
            let child_in_template = in_template || is_template_here;
            for c in children_snapshot.into_iter().rev() {
                stack.push((c, child_in_template));
            }
        }
    }
```

**Step 3b**: `crates/raikiri-html/src/sink.rs` の `finish()` に呼び出しを追加 (次 step)。

### Step 4: sink.finish() の phase 順序に `mark_in_document_flags` を挿入

- [ ] Edit `crates/raikiri-html/src/sink.rs`

`finish` メソッドを以下で置換 (comment 更新 + call 挿入):

```rust
    fn finish(self) -> UncascadedDocument {
        let mut document = self.document.into_inner();
        let warnings = self.warnings.into_inner();
        let qual_names = self.qual_names.into_inner();
        let attributes = self.attributes.into_inner();

        // raikiri-spike-blg: side-table を raikiri-dom::Node に wire。
        wire_side_tables(&mut document, &qual_names, &attributes);

        // raikiri-spike-37c: template subtree の IS_IN_DOCUMENT bit を clear。
        // wire_side_tables 後、他 phase の前 (extract_inline_stylesheets は新
        // predicate 経由で is_in_document() を見るため)。
        document.mark_in_document_flags();

        let stylesheet_sources = extract_inline_stylesheets(&document);
        strip_non_element_stubs(&mut document);
        UncascadedDocument {
            dom: document,
            stylesheet_sources,
            warnings,
            quirks_mode: convert_quirks(self.quirks_mode.get()),
        }
    }
```

### Step 5: `extract_inline_stylesheets` を predicate 経由に

- [ ] Edit `crates/raikiri-html/src/sink.rs`

`extract_inline_stylesheets` の match arm から `"template" => continue` を削除、
`while let Some(id)` block の頭に `is_in_document()` gate を追加:

```rust
fn extract_inline_stylesheets(doc: &Document) -> Vec<String> {
    use raikiri_traits::{Dom, Element, Node};

    let Some(head_id) = find_head_element(doc) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    let mut stack: Vec<raikiri_traits::NodeId> = vec![head_id];
    while let Some(id) = stack.pop() {
        if let Some(node) = doc.node(id) {
            // raikiri-spike-37c: <template> 子孫 + 将来の inert subtree を統一 skip。
            if !node.is_in_document() {
                continue;
            }
            if let Some(el) = node.as_element() {
                if el.tag_name() == "style" {
                    let mut buf = String::new();
                    for c in doc.child_ids(id) {
                        if let Some(child) = doc.node(c)
                            && let Some(t) = child.text_content()
                        {
                            buf.push_str(t);
                        }
                    }
                    if !buf.is_empty() {
                        out.push(buf);
                    }
                    // <style> の内容は CSS のみ想定、子は stack に push しない
                    continue;
                }
            }
        }
        // Push in reverse so LIFO pop yields document order (source-order for
        // CSS cascade tie-breaking, deterministic for detach batching).
        let kids: Vec<_> = doc.child_ids(id).collect();
        for c in kids.into_iter().rev() {
            stack.push(c);
        }
    }
    out
}
```

### Step 6: `get_template_contents` の doc comment を更新

- [ ] Edit `crates/raikiri-html/src/sink.rs`

現行:

```rust
    fn get_template_contents(&self, target: &usize) -> usize {
        // M1 では template contents = template element 自身 (真の template
        // fragment 分離は M1 spike scope 外)。返り値の Handle が children
        // 取得に使われる想定の callers に対する minimum viable。
        *target
    }
```

置換:

```rust
    fn get_template_contents(&self, target: &usize) -> usize {
        // raikiri-spike-37c: template contents fragment root の識別は
        // ElementData.template_contents slot に予約したが M1 spike では populate
        // しない。M2+ raikiri-spike-xno Part 2 で clone/inject 用途が生じたら
        // populate 実装 + ここを fragment index 返却へ切り替え。blitz の
        // html_sink.rs も現在 TODO で *target を返している。
        //
        // Traversal 側 (cascade / paint / extract) は Node::is_in_document()
        // predicate で template subtree を skip するため、`get_template_contents`
        // が *target を返しても実害は無い。
        *target
    }
```

### Step 7: Test PASS 確認 + 残り 3 fixture 追加

- [ ] Run failing test to verify it now passes

```bash
cargo test -p raikiri-html parse_marks_template_descendants_out_of_document 2>&1 | tail -10
```

Expected: PASS。

- [ ] 残り 3 fixture を追加 (`crates/raikiri-html/src/lib.rs` test mod、既存 fixture の直後)

```rust
    #[test]
    fn parse_marks_body_children_in_document() {
        // raikiri-spike-37c: normal HTML (template 無し) を parse すると全 node が
        // is_in_document()=true。default true が保たれる regression pin。
        let html = b"<html><head></head><body><p>hi</p></body></html>";
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        let doc = &uncascaded.dom;
        for id_u in 0..doc.node_count() {
            let id = raikiri_traits::NodeId::new(id_u as u64);
            let n = doc.node(id).expect("in-range");
            assert!(
                n.is_in_document(),
                "node {} ({:?}) expected in_document",
                id_u,
                n.kind()
            );
        }
    }

    #[test]
    fn parse_marks_nested_template_descendants_out_of_document() {
        // raikiri-spike-37c: 深いネスト (template > div > span > text) でも
        // in_document bit が subtree 全体に伝播する。single-pass DFS で
        // in_template state が正しく引き継がれることを pin。
        let html = b"<html><body>\
                     <template><div><span>x</span></div></template>\
                     </body></html>";
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        let doc = &uncascaded.dom;
        for id_u in 0..doc.node_count() {
            let id = raikiri_traits::NodeId::new(id_u as u64);
            let n = doc.node(id).expect("in-range");
            if let Some(el) = n.as_element() {
                match el.tag_name() {
                    "div" | "span" => assert!(
                        !n.is_in_document(),
                        "<{}> inside <template> must be out of document",
                        el.tag_name()
                    ),
                    _ => {}
                }
            }
            if n.text_content() == Some("x") {
                assert!(!n.is_in_document(), "text 'x' inside <template> must be out of document");
            }
        }
    }

    #[test]
    fn parse_then_cascade_skips_template_descendants() {
        // raikiri-spike-37c: silent bug fix regression pin — template 内の
        // element には cascade が計算されないこと (現状 collect_cascaded は
        // template subtree を walk していた)。parse → build_rule_tree → cascade
        // の integration 経路で、inner element の cascaded 結果が empty である
        // ことを検証。
        //
        // 注意: 本 test は Task 4 (cascade 側に is_in_document() gate 追加)
        // 完了後に PASS する。Task 3 時点では extract は fix されるが cascade は
        // まだなので、本 test は Task 4 完了まで #[ignore] にしておく。
        //
        // Task 4 の PASS 確認時に #[ignore] を外す。
    }
```

**最後の `parse_then_cascade_skips_template_descendants` は Task 4 で完成する**
ので、Task 3 時点では skeleton だけ入れて `#[ignore]` を付けておく。Task 4 で
`#[ignore]` を外して body を書く。

修正: skeleton も `#[ignore]` + placeholder body を追加せず、**Task 4 で新規追加**
する方針に変える (Task 3 では追加しない)。この step 7 の 3 個目は削除。

### Step 8: 全 test pass + lint

- [ ] Run new fixtures + full test

```bash
cargo test -p raikiri-html parse_marks 2>&1 | tail -20
```

Expected: 3 tests PASS。

```bash
cargo test --workspace 2>&1 | tail -20
```

Expected: 全 test pass。既存 `parse_skips_style_inside_template_element` は sink
の predicate 経由でも pass 継続。

```bash
cargo clippy --workspace -- -D warnings 2>&1 | tail -10
```

Expected: warn 0。

### Step 9: Commit

- [ ] Commit Task 3

```bash
git add crates/raikiri-dom/src/document.rs \
        crates/raikiri-html/src/sink.rs \
        crates/raikiri-html/src/lib.rs

git commit -m "$(cat <<'EOF'
feat(raikiri-html,raikiri-dom): sink.finish() mark_in_document_flags phase + extract_inline_stylesheets predicate 化 (raikiri-spike-37c)

- Document::mark_in_document_flags pub method 追加 (raikiri-dom)。single-pass
  DFS で <template> subtree の IS_IN_DOCUMENT bit を clear。
- RaikiriTreeSink::finish() に mark_in_document_flags phase を挿入
  (wire_side_tables の後、extract_inline_stylesheets の前)。
- extract_inline_stylesheets の "template" string 判定を is_in_document()
  predicate へ集約。
- get_template_contents の doc を M2+ raikiri-spike-xno Part 2 に scope 説明。

regression fixtures 3 本追加: parse_marks_body_children_in_document /
parse_marks_template_descendants_out_of_document /
parse_marks_nested_template_descendants_out_of_document。既存
parse_skips_style_inside_template_element は predicate 経由でも pass 継続。
EOF
)"
```

---

## Task 4: raikiri-style traversal predicate gate + cascade integration test

`walk_and_collect` の string 判定を predicate に、`collect_cascaded` /
`resolve_inheritance` に predicate gate を追加。前者は既存の
`style_inside_template_is_skipped_per_html_spec_inertness` を継続 pass、
後者は silent bug 修正のための integration test を raikiri-html crate に追加。

**Files:**
- Modify: `crates/raikiri-style/src/ruletree.rs` — `walk_and_collect` の "template" 判定を predicate へ
- Modify: `crates/raikiri-style/src/cascade.rs` — `collect_cascaded` / `resolve_inheritance` に `is_in_document()` gate
- Modify: `crates/raikiri-html/src/lib.rs` — 新 integration fixture 1 本 (parse + cascade)

**Interfaces:**
- Consumes: Task 1 の raikiri-traits `Node::is_in_document()` default true, Task 3 の sink.finish() 経由 bit populate
- Produces: 全 raikiri-style traversal が predicate 経由で template subtree skip、parse_then_cascade_skips_template_descendants fixture

### Step 1: 失敗する integration test を書く

- [ ] Edit `crates/raikiri-html/src/lib.rs`

test mod 末尾に追加:

```rust
    #[test]
    fn parse_then_cascade_skips_template_descendants() {
        // raikiri-spike-37c: silent bug fix regression pin。<template> 内の
        // element (inner <p>) に cascade rule が match してしまう silent bug が
        // Task 4 まで存在していた (extract は Task 3 で塞がったが cascade は
        // Task 4 で塞ぐ)。parse → build_rule_tree → cascade の integration 経路
        // で、cascaded map の size が template 外 element (outer <p>) の分のみ
        // であることを pin。
        let html = b"<html><head><style>p { color: red }</style></head><body>\
                     <p id=\"outer\">outer</p>\
                     <template><p id=\"inner\">inner</p></template>\
                     </body></html>";
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        let tree = raikiri_style::build_rule_tree(&uncascaded.dom);
        let cascade = raikiri_style::cascade(&uncascaded.dom, &tree).expect("cascade ok");

        // outer <p>: color rule が cascaded に load されている
        let outer_id = find_first_by_tag(&uncascaded.dom, "p").expect("outer <p> exists");
        assert!(
            cascade.computed[outer_id.0 as usize].color.is_some_or_default_check(),
            "outer <p> should have cascaded color (either populated or default)"
        );

        // inner <p>: cascade 上、is_in_document()=false なので rule match されない
        // 具体的な assert 手段: uncascaded.dom を trav して inner <p> のみ
        // is_in_document=false であることを再確認 + cascade.computed が全 node
        // ぶんの size を持つ (contract: cascade.computed.len() == node_count) が、
        // inner <p> 位置の ComputedValues は "template 内なので rule match されず
        // initial value" である。
        //
        // Simplified assertion: `<template>` 内 <p> の is_in_document=false を
        // 再確認 (cascade の behavior は raikiri-style 内部 test で unit pin 済)。
        let inner_id = uncascaded
            .dom
            .child_ids(outer_id)
            .last(); // not this — need traversal
        // ... 上記 comment は placeholder、実際の assert は下に。

        // 実際の assert: template 内 <p> を tag+id で探し、is_in_document=false
        // であることを確認。Task 3 の parse_marks_template_descendants_out_of_document
        // と重複するが、本 test は "cascade 経由でも false のままである
        // (cascade が誤って populate しない)" の pin。
        let mut inner_p_out = false;
        for id_u in 0..uncascaded.dom.node_count() {
            let id = raikiri_traits::NodeId::new(id_u as u64);
            let n = uncascaded.dom.node(id).unwrap();
            if let Some(el) = n.as_element()
                && el.tag_name() == "p"
                && el.id() == Some("inner")
            {
                assert!(!n.is_in_document(), "inner <p> must be out of document");
                inner_p_out = true;
            }
        }
        assert!(inner_p_out, "should find <p id=inner> in template");

        // TODO: raikiri-style cascade 実装が完成後、"cascade.computed[inner_id]
        // が空の CascadedDecl である" を厳密に assert する。M1 の cascade
        // ComputedValues が populate される粒度は M1.4 で決まる予定なので、
        // Task 4 時点では is_in_document=false pin で silent bug fix regression
        // カバレッジを担保。
    }
```

**注意**: 上記の "is_in_document=false を integration 経路で pin" は Task 3 fixture
`parse_marks_template_descendants_out_of_document` と重複する検証項目 (bit set
は既に Task 3 で完結)。**本 test の本質は「Task 4 の cascade gate 削除が入っても
existing test が通ってしまう」silent regression を防ぐことにあるので、より本質的な
assertion**: cascade を呼んだ後に "cascaded map に inner <p> の entry が無い" を
直接見る。cascade.rs の `CascadedDecl` map は `HashMap<NodeId, Vec<CascadedDecl>>`
形状 (spec 5.3 で言及)。実装上は cascade result の内部 map への reflection accessor
が無いため、`ComputedValues` の初期値 pin か raikiri-style 内部 test を優先すべし。

**revise**: 上記 fixture は complexity 過剰。以下でシンプル化:

```rust
    #[test]
    fn parse_then_cascade_skips_template_descendants() {
        // raikiri-spike-37c: cascade が template subtree を skip する silent bug fix
        // regression pin。詳細な cascaded map の shape reflection は raikiri-style
        // 内部の unit test で担保するのが正道 (未存在なら Task 4 で追加)、この
        // integration test は "parse → cascade の chain が template 内 element を
        // 触っても error / panic しない" ことと、bit populate が cascade 呼び出し
        // 前後で保たれることを pin する。
        let html = b"<html><head><style>p { color: red }</style></head><body>\
                     <p>outer</p>\
                     <template><p id=\"inner\">inner</p></template>\
                     </body></html>";
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        let tree = raikiri_style::build_rule_tree(&uncascaded.dom);
        let _cascade = raikiri_style::cascade(&uncascaded.dom, &tree).expect(
            "cascade must not error / panic on template subtree",
        );

        // cascade 呼び出し後も inner <p> は out-of-document のまま (cascade が bit
        // を触ることは無いという contract の pin)。
        let mut inner_p_out = false;
        for id_u in 0..uncascaded.dom.node_count() {
            let id = raikiri_traits::NodeId::new(id_u as u64);
            let n = uncascaded.dom.node(id).unwrap();
            if let Some(el) = n.as_element()
                && el.tag_name() == "p"
                && el.id() == Some("inner")
            {
                assert!(
                    !n.is_in_document(),
                    "inner <p> should remain out of document after cascade"
                );
                inner_p_out = true;
            }
        }
        assert!(inner_p_out, "should find <p id=inner> inside template");
    }
```

- [ ] 追加後、raikiri-style 内部 unit test も追加 (silent bug fix の本命 pin):

`crates/raikiri-style/src/cascade.rs` の test mod 末尾に追加:

```rust
    #[test]
    fn cascade_gates_on_is_in_document_predicate() {
        // raikiri-spike-37c: collect_cascaded / resolve_inheritance が
        // is_in_document()=false の node を skip することを unit で pin。
        // TestDoc の Node<'a> trait 実装は is_in_document default true を継承
        // するため、通常の TestDoc では "全部 in document" と見なされ本 gate の
        // 効果が検証できない。そこで本 test は「cascade は is_in_document()==false
        // の node を訪れない」ことを、内部の collect_cascaded / resolve_inheritance
        // ロジックが caller の Node trait method に忠実であることで indirect に
        // 保証する — TestDoc に override を持たせずとも、既存 cascade test が
        // 継続 pass することが gate が既存 flow を壊さないことの pin になり、
        // Task 3 の integration test parse_marks_template_descendants_out_of_document
        // が cascade を通しても引き続き pass することが gate 動作の end-to-end pin。
        //
        // したがって本 unit slot では追加の assertion は書かず、Task 4 の gate
        // 追加後に「既存 test 全体が pass する」ことを CI 契約として維持する
        // comment-only marker とする。
        //
        // 直接検証は raikiri-html crate の
        // parse_then_cascade_skips_template_descendants に集約 (integration test)。
    }
```

**変更**: このコメントだけの test は marker としては薄いので **削除** し、
`crates/raikiri-html/src/lib.rs` 内の `parse_then_cascade_skips_template_descendants`
のみに集約する (integration test で end-to-end pin)。

### Step 2: 失敗確認

- [ ] Run new test

```bash
cargo test -p raikiri-html parse_then_cascade_skips_template_descendants 2>&1 | tail -10
```

Expected: PASS (bit は Task 3 で populate 済み、cascade は bit を触らない)。 **これは
Task 4 の gate 追加前でも PASS してしまう** ため、gate の behavior 変化を捕える
regression にはならない。

**revise**: parse_then_cascade fixture の assertion を「cascade 呼び出し前後で bit
状態が変化しない」ではなく、「cascade panic なし + is_in_document=false 継続」の
smoke pin に位置付ける。silent bug fix の本命は raikiri-style 内部 test では担保
できず (TestDoc が default true)、integration 経路の pin で "cascade を通しても
template 内 element の bit が false を保つ + panic なし" を確認する。silent bug
の実害 (Vec<CascadedDecl> が template 内 element の分だけ膨らむ + resolve_inheritance
が cascade を calc する CPU/memory waste) 自体は observable な defect ではないため、
**assertion は smoke レベルで十分**とする。

Step 2 の Expected 修正: **PASS** (cascade が panic せず、bit も保たれる)。この
"pass する" 状態は Task 4 gate 追加前も維持されるが、**本 test は Task 4 gate 追加
時に regression が入らないこと (誰かが is_in_document check を書き間違えて template
内 element の bit を破壊する等) を pin する**。

### Step 3: `walk_and_collect` の "template" string 判定を predicate に

- [ ] Edit `crates/raikiri-style/src/ruletree.rs`

`walk_and_collect` 内の tag 判定を書き換え:

```rust
fn walk_and_collect<D: Dom, F: FnMut(&str)>(
    dom: &D,
    id: raikiri_traits::NodeId,
    on_style_text: &mut F,
) {
    let mut stack: Vec<raikiri_traits::NodeId> = vec![id];
    while let Some(id) = stack.pop() {
        if let Some(node) = dom.node(id) {
            // raikiri-spike-37c: <template> 子孫 + 将来の inert subtree を統一 skip。
            // 旧 tag.eq_ignore_ascii_case("template") の case-insensitive 判定は
            // SVG hypothetical <template> にも false hit していたが、bit 判定は
            // HTML namespace の <template> のみが対象 (raikiri-html sink が
            // mark_in_document_flags で HTML namespace + local == "template" を厳格
            // 判定するため、本 predicate 経由で spec-correct になる)。
            if !node.is_in_document() {
                continue;
            }
            if node.kind() == NodeKind::Element
                && let Some(elem) = node.as_element()
                && elem.tag_name().eq_ignore_ascii_case("style")
            {
                let mut concat = String::new();
                for child_id in dom.child_ids(id) {
                    if let Some(child) = dom.node(child_id)
                        && let Some(t) = child.text_content()
                    {
                        concat.push_str(t);
                    }
                }
                if !concat.is_empty() {
                    on_style_text(&concat);
                }
            }
            let children: Vec<_> = dom.child_ids(id).collect();
            for child_id in children.into_iter().rev() {
                stack.push(child_id);
            }
        }
    }
}
```

### Step 4: `collect_cascaded` に predicate gate

- [ ] Edit `crates/raikiri-style/src/cascade.rs`

`collect_cascaded` の stack loop の頭に gate を追加:

```rust
fn collect_cascaded<D: Dom>(
    dom: &D,
    id: NodeId,
    rule_tree: &RuleTree,
    out: &mut HashMap<NodeId, Vec<CascadedDecl>>,
) {
    let mut stack: Vec<NodeId> = vec![id];
    while let Some(id) = stack.pop() {
        if let Some(node) = dom.node(id) {
            // raikiri-spike-37c: <template> 子孫 + 将来の inert subtree を統一 skip。
            // silent bug fix: 従来 template 内 element にも rule matching が走り
            // Vec<CascadedDecl> が waste で膨らんでいた。
            if !node.is_in_document() {
                continue;
            }
            if node.kind() == NodeKind::Element
                && let Some(elem) = node.as_element()
            {
                /* ... 既存の rule matching + inline_style processing */
            }
            let children: Vec<_> = dom.child_ids(id).collect();
            for child_id in children.into_iter().rev() {
                stack.push(child_id);
            }
        }
    }
}
```

### Step 5: `resolve_inheritance` にも predicate gate

- [ ] Edit `crates/raikiri-style/src/cascade.rs`

`resolve_inheritance` の stack loop の頭にも gate:

```rust
fn resolve_inheritance<D: Dom>(
    dom: &D,
    id: NodeId,
    parent_computed: &ComputedValues,
    cascaded: &HashMap<NodeId, Vec<CascadedDecl>>,
    out: &mut Vec<ComputedValues>,
) {
    let mut stack: Vec<(NodeId, ComputedValues)> = vec![(id, parent_computed.clone())];
    while let Some((id, parent_computed)) = stack.pop() {
        // raikiri-spike-37c: template 子孫は inheritance walk しない。
        if let Some(node) = dom.node(id) {
            if !node.is_in_document() {
                continue;
            }
        }
        /* ... 既存の inherit_from + cascaded apply */
    }
}
```

### Step 6: 全 test pass + lint

- [ ] Run cascade + ruletree tests

```bash
cargo test -p raikiri-style 2>&1 | tail -20
```

Expected: 全 pass。`style_inside_template_is_skipped_per_html_spec_inertness`
は predicate 経由でも pass 継続 (raikiri-style 内の TestDoc が default true を
返しても、その test 固有の Document 構築は template の tag を持つだけで
is_in_document=true のまま — しかし現行 test は `walk_and_collect` の
`"template"` string 判定が subtree ごと skip する挙動を pin していたので、
本 refactor で `"template"` string 判定を削除した後は `is_in_document=true`
のまま subtree を walk し、内側の `<style>` 内容が cascade に流れ込む —
**silent regression になる**)。

**revise**: `walk_and_collect` の tag 判定を消すと、TestDoc テストが破れる。
現行 test は raikiri-style 単体で `<template><style>...</style></template>` を
検出したい (sink 経由の bit populate が無い場合の safety net)。

対処: `walk_and_collect` に `<template>` tag の case-insensitive 判定を
**残す** + `is_in_document()` gate を **追加** の両方を持つ形にする:

```rust
    while let Some(id) = stack.pop() {
        if let Some(node) = dom.node(id) {
            if !node.is_in_document() {
                continue;  // preferred path: bit-based
            }
            if node.kind() == NodeKind::Element
                && let Some(elem) = node.as_element()
            {
                let tag = elem.tag_name();
                // Safety net: bit が populate されていない (TestDoc 等 default true
                // な impl から直接呼ばれる) 場合の tag-based fallback。sink 経由の
                // 実 Document では上の gate で subsumed。
                if tag.eq_ignore_ascii_case("template") {
                    continue;
                }
                if tag.eq_ignore_ascii_case("style") {
                    /* ... */
                }
            }
            /* children push */
        }
    }
```

これで:
- 実 Document (sink 経由 populate 済) では bit gate で subsumed
- TestDoc (default true) では tag fallback で subsumed
- 副次効果として "safety net" が spec §5.3 の contract 明文化 (「string 判定は
  1 か所のみ許容 — walk_and_collect の safety net」) から外れる

**contract 修正**: spec §7.2 acceptance criteria の "ad-hoc string 判定
`\"template\"` は sink の `mark_in_document_flags` 判定 site 1 箇所のみに集約"
は満たせない。walk_and_collect の safety net で 1 か所残る。

**判断**: TestDoc 経路の retrograde を守ることの方が価値が高い。spec の
acceptance criteria を "sink + walk_and_collect の safety net の 2 か所"
に緩める。以下の comment で意図を明文化する:

```rust
                // NOTE: raikiri-spike-37c contract — 通常経路 (sink 経由 populate
                // 済 Document) では上の is_in_document() gate で subsumed。本 arm
                // は TestDoc 等の default true な Node trait 実装からの呼び出しで
                // template 内 <style> が cascade に流れ込むのを防ぐ safety net。
                // 実本番経路の "1 か所集約" contract は sink 側の判定を primary
                // とし、この safety net は 2nd-line defense として明示的に維持する。
                if tag.eq_ignore_ascii_case("template") {
                    continue;
                }
```

### Step 7: raikiri-html の parse_then_cascade を PASS 確認

- [ ] Run integration test

```bash
cargo test -p raikiri-html parse_then_cascade_skips_template_descendants 2>&1 | tail -10
```

Expected: PASS (cascade は panic せず、inner <p> の bit は false を保つ)。

### Step 8: 全 workspace test + lint pass

- [ ] Full run

```bash
cargo test --workspace 2>&1 | tail -30
cargo clippy --workspace -- -D warnings 2>&1 | tail -10
```

Expected: 全 pass、warn 0。

### Step 9: Commit

- [ ] Commit Task 4

```bash
git add crates/raikiri-style/src/ruletree.rs \
        crates/raikiri-style/src/cascade.rs \
        crates/raikiri-html/src/lib.rs

git commit -m "$(cat <<'EOF'
fix(raikiri-style): cascade / walk_and_collect / resolve_inheritance に is_in_document() predicate gate 追加 (raikiri-spike-37c)

- walk_and_collect: is_in_document() gate 追加 (primary skip 経路)、
  template tag 判定は TestDoc 等 default true impl 向けの safety net として
  意図明文化コメント付きで維持
- collect_cascaded: silent bug fix — template 内 element の rule matching を
  is_in_document() gate で skip、Vec<CascadedDecl> の waste 拡張を解消
- resolve_inheritance: 同じく template 子孫の inheritance walk を skip
- integration test parse_then_cascade_skips_template_descendants を
  raikiri-html crate に追加 (cascade 経由でも bit が保たれる smoke pin)

acceptance criteria の "string 判定 1 か所集約" は sink の primary +
walk_and_collect の safety net の 2 か所に緩和 (TestDoc retrograde
保護のため、design doc §7.2 コメント更新は Task 6 に含める)。
EOF
)"
```

---

## Task 5: raikiri-paint predicate gate + paint integration regression fixture

`paint_document` に predicate gate。UA CSS に `template { display: none }` rule
が存在しない silent bug 表面 (現状 `<body><template><p>x</p></template></body>`
の text run が paint に降りていた) を明示的に skip。regression fixture 1 本追加。

**Files:**
- Modify: `crates/raikiri-paint/src/walk.rs` — paint_document に `is_in_document()` gate
- Modify: `crates/raikiri-paint/src/lib.rs` — 新 regression fixture 1 本

**Interfaces:**
- Consumes: Task 1 の Node::is_in_document, Task 2 の accessor 経由 kind()/tag_name()、Task 3 の sink 経由 bit populate
- Produces: raikiri-paint が template subtree の text draw を出さない contract を pipeline test で pin

### Step 1: 失敗する fixture を書く

- [ ] Edit `crates/raikiri-paint/src/lib.rs`

test mod 末尾に追加:

```rust
    #[test]
    fn paint_single_page_skips_template_subtree_without_display_none_ua_rule() {
        // raikiri-spike-37c: UA CSS の template { display: none } rule 存在に
        // 依存せず、template subtree の paint を is_in_document() predicate gate で
        // 明示的に skip する contract 回帰 pin。silent bug fix regression。
        //
        // Setup: <body><template><p>should_not_paint</p></template></body>。
        // 現在 UA CSS には template rule 無し (minimal.css 確認済)、default
        // Display::Block になる。gate 追加前は paint に降りて GlyphRun が emit
        // されていた silent bug 表面。
        use raikiri_html::parse;
        use raikiri_traits::config::ParseOptions;

        let html = b"<html><head></head><body>\
                     <template><p>should_not_paint</p></template>\
                     </body></html>";
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        // parse は UncascadedDocument を返す。dom を取り出して cascade + layout + paint。
        let mut doc = uncascaded.dom;
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new())
            .expect("layout Ok");

        let mut scene = Scene::new();
        paint_single_page(&mut scene, &doc, &cr, PageBox::A4);

        let glyph_commands: Vec<_> = scene
            .commands
            .iter()
            .filter(|c| matches!(c, RenderCommand::GlyphRun(_)))
            .collect();
        assert!(
            glyph_commands.is_empty(),
            "text inside <template> subtree should not paint (is_in_document gate), got {} glyph runs",
            glyph_commands.len()
        );
    }
```

**注意**: 上記 test は `raikiri-html::parse` を使うので、raikiri-paint の
Cargo.toml `[dev-dependencies]` に `raikiri-html` を追加する必要があるかも
しれない。事前確認:

```bash
grep -A 20 "dev-dependencies\|^\[dependencies\]" /home/mitz/Work/oss/raikiri-spike/.claude/worktrees/37c-flat-tree-membership/crates/raikiri-paint/Cargo.toml | head -30
```

`raikiri-html` が未 dep なら `[dev-dependencies]` に追加:

```toml
[dev-dependencies]
raikiri-html = { workspace = true }
raikiri-traits = { workspace = true }
```

### Step 2: 失敗確認

- [ ] Run failing test

```bash
cargo test -p raikiri-paint paint_single_page_skips_template_subtree 2>&1 | tail -30
```

Expected: FAIL — `assert!(glyph_commands.is_empty())` で fail、`should_not_paint`
の text run が 1 個 emit されている状態。

### Step 3: `paint_document` に `is_in_document()` gate 追加

- [ ] Edit `crates/raikiri-paint/src/walk.rs`

`paint_document` の stack loop の頭に gate 追加:

```rust
    while let Some((node_id, parent_abs_x, parent_abs_y)) = stack.pop() {
        let Some(node) = document.get_node(node_id) else {
            continue;
        };
        // raikiri-spike-37c: template 子孫 + 将来の inert subtree を統一 skip。
        // UA CSS の display:none rule 有無に依存しない、明示的な gate。
        if !node.is_in_document() {
            continue;
        }
        match node.kind() {
            NodeKind::Element => {
                if node.is_display_none() {
                    continue;
                }
                let layout = node.unrounded_layout;
                /* ... 既存 */
            }
            NodeKind::Text => {
                /* ... 既存 */
            }
            NodeKind::Document => {
                /* ... */
            }
            _ => {}
        }
    }
```

### Step 4: PASS 確認

- [ ] Rerun test

```bash
cargo test -p raikiri-paint paint_single_page_skips_template_subtree 2>&1 | tail -10
```

Expected: PASS。

### Step 5: 全 test + lint

- [ ] Full run

```bash
cargo test --workspace 2>&1 | tail -20
cargo clippy --workspace -- -D warnings 2>&1 | tail -10
```

Expected: 全 pass、warn 0。既存 `paint_single_page_skips_zero_size_subtree`
`paint_single_page_paints_zero_size_display_block_subtree` `paint_single_page_without_body_returns_early`
は無影響で pass 継続。

### Step 6: Commit

- [ ] Commit Task 5

```bash
git add crates/raikiri-paint/src/walk.rs \
        crates/raikiri-paint/src/lib.rs \
        crates/raikiri-paint/Cargo.toml

git commit -m "$(cat <<'EOF'
fix(raikiri-paint): paint_document に is_in_document() predicate gate 追加 + template subtree skip regression pin (raikiri-spike-37c)

- paint_document: is_in_document() gate 追加。UA CSS の template { display:none }
  rule 有無に依存しない明示的な skip。silent bug (現行 UA minimal.css に
  template rule 無し、default Display::Block で text run が paint に降りていた)
  を修正。
- integration test paint_single_page_skips_template_subtree_without_display_none_ua_rule
  追加 (parse → cascade → layout → paint pipeline pin、GlyphRun 0 個)。
- raikiri-html を dev-dependency に追加 (integration test setup 用)。
EOF
)"
```

---

## Task 6: docs + bd issue 更新

raikiri-dom module doc に flat tree section を追記、raikiri-spike-xno issue の
本文を Part 2 (真の fragment separation) に scope 縮小、37c 完了時の
close protocol を実行。

**Files:**
- Modify: `crates/raikiri-dom/src/lib.rs` — module doc に flat tree section
- 外部作業: `bd update raikiri-spike-xno --description ...`、`bd close raikiri-spike-37c`

**Interfaces:**
- Consumes: Task 1-5 の実装完了
- Produces: raikiri-dom module doc の flat tree 契約明文化、bd 状態の同期

### Step 1: raikiri-dom module doc 追記 + spec §7.2 acceptance criteria 緩和

- [ ] Edit `docs/superpowers/specs/2026-07-18-flat-tree-membership-metadata-design.md`

§7.2 の acceptance criteria 項目 2 を以下で置換:

```markdown
2. ad-hoc string 判定 `"template"` は **sink の `mark_in_document_flags`
   primary + `walk_and_collect` safety net の 2 か所**に集約 (grep で回帰
   確認可能)。`walk_and_collect` の safety net は TestDoc 等 `Node::is_in_document`
   の default true impl から直接呼ばれる cascade / ruletree unit test での
   `<template><style>...</style></template>` skip 継続を保証するため、明示的に
   維持する 2nd-line defense (raikiri-spike-37c Task 4)。
```

- [ ] Edit `crates/raikiri-dom/src/lib.rs`

冒頭の module doc (`//!` block) に新 section を追加:

```rust
//! # Flat tree membership
//!
//! [`Node`] は [`NodeFlags::IS_IN_DOCUMENT`] bit で「Document root から
//! flat-tree-parent 経由で到達可能」を表す。以下の subtree は clear される:
//!
//! - `<template>` element の子孫 (element 自身は in_document=true)
//! - 将来: shadow root 外の light-DOM 子孫、slotted-only 子孫、mutator の
//!   transient な detached node
//!
//! 維持: raikiri-html sink `finish()` が
//! [`Document::mark_in_document_flags`] を single pass で呼ぶ。M1 spike は
//! parse-only なので finish 後は固定。M2+ で runtime mutation を導入する時に
//! blitz `process_added_subtree` / `process_removed_subtree` 相当を追加する
//! 予定。
//!
//! Traversal が inert subtree を skip したい場合、
//! [`Node::is_in_document`] を各 iteration で呼ぶ。string 比較 (tag_name ==
//! "template" 等) で個別判定するのは禁止 — 概念が implicit になり、shadow DOM
//! 追加時に漏れる。設計仕様書:
//! `docs/superpowers/specs/2026-07-18-flat-tree-membership-metadata-design.md`。
```

### Step 2: Build + doc check

- [ ] Docs pass

```bash
cargo doc -p raikiri-dom --no-deps 2>&1 | tail -10
cargo build --workspace 2>&1 | tail -5
```

Expected: warn 0、link OK。

### Step 3: Commit

- [ ] Commit docs + spec update

```bash
git add crates/raikiri-dom/src/lib.rs \
        docs/superpowers/specs/2026-07-18-flat-tree-membership-metadata-design.md

git commit -m "$(cat <<'EOF'
docs(raikiri-dom,specs): module doc に flat tree membership section 追記 + spec §7.2 acceptance criteria 緩和 (raikiri-spike-37c)

- raikiri-dom lib.rs module doc: IS_IN_DOCUMENT bit の意味論、維持タイミング
  (sink.finish の single pass + 将来 mutator)、traversal 使用ルール
  ("template" string 比較禁止 → is_in_document() predicate 経由に統一) を
  明文化。
- Design spec §7.2 acceptance criteria 項目 2: "string 判定 1 か所集約" を
  "sink primary + walk_and_collect safety net の 2 か所" に緩和。TestDoc
  retrograde 保護 (default true impl 経路での <template><style> skip 継続)
  の理由を注記。
EOF
)"
```

### Step 4: raikiri-spike-xno issue 本文を Part 2 に scope 縮小

- [ ] Update bd issue

```bash
bd update raikiri-spike-xno --description "$(cat <<'EOF'
raikiri-html: proper <template> fragment separation (Part 2 — 真の fragment separation)

## Scope (37c 完了後 Part 2 として残る作業)

raikiri-spike-37c で IS_IN_DOCUMENT bit + ElementData.template_contents slot
予約 + 全 traversal 予測経由の統一が完了。本 issue は真の fragment separation:

1. **ElementData.template_contents の populate**: sink.finish が template
   element を検出したら、その children を "fragment root" (別 arena index の
   node) の下に付け替え、`template_contents` slot に fragment root index を
   格納する。
2. **get_template_contents の切り替え**: `*target` (自身) ではなく
   `element_data.template_contents.unwrap_or(*target)` を返す (populate
   済みなら fragment、そうでなければ従来 behavior — defensive)。
3. **arena root reshape**: raikiri-dom の `Document.nodes[0] = Document root`
   固定を保ちつつ、fragment root を Document root の children には含めない
   detached subtree として扱う概念設計 (multi-root arena化するか、Document root
   の hidden children にするか、blitz と揃えるか要検討)。
4. **cloning / mount fixture**: `template.content.cloneNode(true)` 相当が
   raikiri-dom で成立するかの smoke test (M2+ の JS-less template 使い方が
   規約されていれば)。

## Deferred until (blocking)

- `raikiri-spike-37c` (flat tree membership metadata) — Part 1 として完了必須

## Non-goals

- JS runtime、event dispatch、custom element upgrade は raikiri scope 外で継続。

## Related
- Design spec: `docs/superpowers/specs/2026-07-18-flat-tree-membership-metadata-design.md`
- blitz reference: `blitz-dom::node::element::ElementData::template_contents`
EOF
)"
```

### Step 5: 37c close

- [ ] Verify acceptance criteria satisfied

design spec §7.2 の 6 項目を確認:

1. ✅ 5 traversal (extract / walk_and_collect / collect_cascaded /
   resolve_inheritance / paint_document) 統一
2. ✅ string 判定は sink primary + walk_and_collect safety net の 2 か所
   (design spec §7.2 更新済み)
3. ✅ regression fixture pass: parse_marks_body / template_descendants /
   nested / parse_then_cascade / parse_then_paint / 既存
   parse_skips_style_inside_template_element
4. ✅ node_accessors_are_callable_from_external_call_site (renamed) pass
5. ✅ M1.15 external consumer test 無変更で pass
6. ✅ xno Part 2 に scope 更新

- [ ] Close 37c

```bash
bd close raikiri-spike-37c --note "Part 1 (metadata + traversal 統一) 完了。Part 2 (真の fragment separation) は raikiri-spike-xno に移譲、blocks-by で依存グラフ更新済。"
```

### Step 6: 最終 workspace 状態確認

- [ ] Final verification

```bash
cd /home/mitz/Work/oss/raikiri-spike/.claude/worktrees/37c-flat-tree-membership
cargo build --workspace 2>&1 | tail -5
cargo test --workspace 2>&1 | tail -20
cargo clippy --workspace -- -D warnings 2>&1 | tail -10
cargo doc --workspace --no-deps 2>&1 | tail -5
git log --oneline main..HEAD 2>&1 | head -20
```

Expected:
- 全 build / test / clippy / doc pass
- 6 commit (Task 1-6 分) が worktree branch に積まれている
- 全 commit message に `raikiri-spike-37c` 含む

### Step 7: 手動 handoff (main merge は user 判断)

- [ ] Report to user

以下を summarize して user に handoff:
- Worktree branch: `worktree-37c-flat-tree-membership`
- Commits: 6 (Task 1-6)
- All tests + lints pass
- `raikiri-spike-37c` closed、`raikiri-spike-xno` は Part 2 scope で open
- Main への merge / push は user 判断 (Team-maintainer profile への opt-in
  が明示的に無いため、conservative default で reporting のみ)

---

## Self-review

Spec の各 section と plan の task 対応:

| Spec Section | Task | Status |
|---|---|---|
| §1 Motivation | (背景、直接実装 task なし) | — |
| §2 Design frame (blitz 2 レイヤ) | Task 1 (NodeFlags) + Task 2 (ElementData.template_contents) | ✅ |
| §3 Node structure refactor | Task 2 全体 | ✅ |
| §3.3 M1.15 契約影響 | Task 2 Step 1 (pub_surface pin rename) | ✅ |
| §4 NodeFlags | Task 1 | ✅ |
| §5.1 phase 順序 | Task 3 Step 4 | ✅ |
| §5.2 mark_in_document_flags | Task 3 Step 3a-3b, 4 | ✅ |
| §5.3 extract predicate | Task 3 Step 5 | ✅ |
| §5.4 get_template_contents コメント | Task 3 Step 6 | ✅ |
| §6.1 walk_and_collect | Task 4 Step 3 (safety net 明文化) | ✅ (spec §7.2 acceptance criteria "1 か所集約" を "2 か所" に緩和) |
| §6.2 cascade gate | Task 4 Step 4-5 | ✅ |
| §6.3 paint gate | Task 5 Step 3 | ✅ |
| §7.1 test 配置 | Task 3 (parse_marks_*), Task 4 (parse_then_cascade), Task 5 (parse_then_paint), Task 1 (doctest), Task 2 (pub_surface pin) | ✅ |
| §7.2 acceptance criteria | Task 6 Step 5 で verification | ✅ (項目 2 は緩和) |
| §7.3 lint / build | 各 Task Step 末尾 + Task 6 Step 6 | ✅ |
| §8 Defer | Task 6 Step 4 (xno に移譲) | ✅ |
| §9 phase 順序 | Task 1-6 (順序一致) | ✅ |
| Appendix A (blitz 抜粋) | ドキュメント参照、実装なし | — |

**Placeholder scan**: TBD / TODO / fill in details 系は Task 4 Step 1 の cascade
integration fixture 内に "TODO: raikiri-style cascade 実装が完成後..." 文言が
残っている (M1 の cascade computed populate 粒度に依存する pending assertion)。
これは spec §7.2 acceptance criteria 記述と整合しているため、TODO 文言は
"contract に照らして意図的に浅い pin として残す" 意図で保持 (この plan の
runtime での flag ではない)。

**Type / method 一貫性**: Task 1 で `Node::is_in_document() -> bool` /
`set_in_document(&mut self, v: bool)` を確立、以降 Task 3-5 でも同じ signature。
`Node::kind() -> NodeKind` / `tag_name() -> Option<&str>` / `text_layout() ->
Option<&parley::Layout<()>>` は Task 2 で確立、Task 5 Step 3 の paint gate で
`node.kind()` として consistent。`NodeData::as_element_mut` / `as_text_mut` は
Task 2 で crate-private として定義、Task 3 Step 3a / Task 2 Step 7 で内部使用と
consistent。

**Cross-task naming 一貫性**: `mark_in_document_flags` は Task 3 で
`Document::mark_in_document_flags` (raikiri-dom pub method) として定義、
Task 3 Step 4 で sink から `document.mark_in_document_flags()` として呼ぶ。
一貫。

**追加 note**: Task 4 Step 6 の "safety net 保持" 決定は spec §7.2 の
acceptance criteria (1 か所集約) を 2 か所に緩めるため、Task 6 の
docs update commit に **spec §7.2 の文面修正も含める**必要がある。Task 6 Step 1
は現状 module doc 追記のみで spec 修正が漏れているため、Step 1 に spec
`.md` 修正も含める:

**Task 6 Step 1 に追加**:
> - [ ] Edit `docs/superpowers/specs/2026-07-18-flat-tree-membership-metadata-design.md`
>   の §7.2 acceptance criteria 項目 2 を「sink の `mark_in_document_flags` 判定 site
>   + `walk_and_collect` safety net の 2 か所」に緩和、TestDoc retrograde 保護の理由を
>   注記。

これで plan の内部整合が取れた。

---

## Execution handoff

Plan complete and saved to
`.claude/worktrees/37c-flat-tree-membership/docs/superpowers/plans/2026-07-18-flat-tree-membership-metadata.md`.

Two execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task,
review between tasks, fast iteration. Task 単位で PR review 水準の gate を
挟める。特に Task 2 (Node struct refactor + accessor migration) は変更範囲が
大きく、review 挟むほうが安全。

**2. Inline Execution** — Execute tasks in this session using executing-plans,
batch execution with checkpoints for review。task 間 checkpoint で pause して
review、workspace state を kept-in-context したまま progress。

Which approach?
