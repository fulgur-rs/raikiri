# Flat tree membership metadata + template subtree semantic unification

- **bd issue**: raikiri-spike-37c
- **Related (blocked by this)**: raikiri-spike-xno (Part 2: true fragment separation)
- **Author**: Mitsuru Hayasaka
- **Date**: 2026-07-18
- **Status**: Draft — awaiting review

## 1. Motivation

`<template>` element の subtree は HTML spec 上 inert であり、traversal 系
処理 (stylesheet 抽出、cascade、paint) から skip される必要がある。現状の
raikiri-spike には以下の状態がある:

| 場所 | 実装 | 状態 |
|---|---|---|
| `raikiri-html/src/sink.rs` `extract_inline_stylesheets` | tag == "template" で `continue` | 塞がっている (2026-07-16 commit b4b9364) |
| `raikiri-style/src/ruletree.rs` `walk_and_collect` | `eq_ignore_ascii_case("template")` で `continue` | 塞がっている (大文字/小文字ゆるめ、SVG hypothetical false hit あり) |
| `raikiri-style/src/cascade.rs` `collect_cascaded` | **skip なし** | silent bug — template 内 element に cascade 計算が走る |
| `raikiri-paint/src/walk.rs` `paint_document` | **skip なし** (`is_display_none` 経由に暗黙依存) | silent bug — UA CSS に `template { display: none }` rule が無いため paint に降りる可能性 |
| `raikiri-html/src/sink.rs` `get_template_contents` | `*target` を返す | HTML5 spec 上は separate document fragment を要求、M2+ defer |

問題は「もう 1 か所 skip を足す」ことではなく、"traversal から見える subtree
membership" を表す **意味論の primitive が不在** である点。string 比較で
"template" を各 caller が独自判定しており、規約が implicit で漏れが再発
しやすい (paint と cascade で既に漏れている疑いが確認された)。

## 2. Design frame

### 2.1 Blitz の 2 レイヤ構造 (先例調査)

blitz は同じ問題に対し **traversal metadata** と **構造分離 slot** の 2
concept を分業で提供している:

```
(1) Node.flags: NodeFlags
      const IS_IN_DOCUMENT = 1 << 2
      //  "has a parent and isn't a template node"

    · Mutator が subtree 追加/削除時に DFS で set/unset
    · 使用箇所: snapshot / id-map / style ops / damage / animation
    · 実質「flat tree の一員か」を表す一段抽象 (spec 用語より緩い)

(2) ElementData.template_contents: Option<usize>
    · Element data 上の "sidecar" pointer
    · template contents fragment root (arena 内の別 subtree) を指す
    · blitz-html sink も現状は `*target` を返している (TODO)
    · slot だけ予約、populate は将来
```

両者は独立に使用可能:
- (1) だけあれば traversal 側の skip 判定は完結 (predicate 1 個で全 caller 統一)
- (2) だけあれば `template.content` の cloning / mount identity が
  取れる (raikiri M1 では不要)

**Blitz 自身も (1) は稼働、(2) は slot 予約のみ** という段階的アプローチを
採っている。raikiri もこの分業をそのまま採用する (backport nominal cost)。

### 2.2 意味論 contract

```
IS_IN_DOCUMENT bit の意味:
  Document root から flat-tree-parent 経由で到達可能な Node

  set:
    · Document root と、その flat-tree 子孫すべて
    · <template> element 自身 (= flat tree 上に存在する node)

  clear:
    · <template> element の子孫 (別 fragment 相当)
    · 将来: shadow root 外の light-DOM 子孫、slot 割当てられない host 直下
    · 将来: transient な detached node (mutator 中の一時状態)

Traversal 側は `is_in_document()` predicate 1 個で subtree 判定を統一する。
tag_name string 比較で個別判定するのは禁止 (概念が implicit になり
shadow DOM 追加時に漏れる)。
```

### 2.3 raikiri M1 での維持タイミング

blitz は runtime mutation を持つため mutator が bit を維持する。raikiri
M1 は parse-only なので、**sink.finish() の single pass DFS** で bit を
set/clear すれば済む。M2+ で mutation runtime が入るときに blitz と同じ
`process_added_subtree` / `process_removed_subtree` 相当を追加する
(将来の仕事、この design の scope 外)。

## 3. Node structure refactor

現行の flat struct から blitz-nominal な tagged union に移行する。

### 3.1 Node struct 変更

**Before**:
```rust
pub struct Node {
    pub(crate) style: Style,
    pub children: Vec<usize>,
    pub(crate) cache: Cache,
    pub unrounded_layout: Layout,
    pub kind: NodeKind,                            // M1.15 pinned pub
    pub tag_name: Option<SmolStr>,                 // M1.15 pinned pub
    pub(crate) text_content: Option<SmolStr>,
    pub(crate) inline_style: Option<SmolStr>,
    pub(crate) namespace: Option<SmolStr>,
    pub(crate) attributes: Vec<Attr>,
    pub text_layout: Option<parley::Layout<()>>,   // M1.15 pinned pub
}
```

**After**:
```rust
pub struct Node {
    pub(crate) style: Style,
    pub children: Vec<usize>,                      // pub 継続
    pub(crate) cache: Cache,
    pub unrounded_layout: Layout,                  // pub 継続
    pub(crate) flags: NodeFlags,                   // NEW
    pub(crate) data: NodeData,                     // NEW
}

#[derive(Debug)]
pub enum NodeData {
    Element(Box<ElementData>),
    Text(TextData),
    Document,
}

#[derive(Debug)]
pub struct ElementData {
    pub(crate) tag_name: SmolStr,
    pub(crate) inline_style: Option<SmolStr>,
    pub(crate) namespace: Option<SmolStr>,
    pub(crate) attributes: Vec<Attr>,
    /// blitz `ElementData.template_contents` に相当する構造分離 slot。
    /// M1 spike では sink が populate せず、`get_template_contents` は
    /// `*target` を返す。M2+ で fragment root index を格納する予定
    /// (raikiri-spike-xno)。
    pub(crate) template_contents: Option<usize>,
}

#[derive(Debug)]
pub struct TextData {
    pub(crate) text_content: SmolStr,
    pub text_layout: Option<parley::Layout<()>>,   // pub 継続 (paint hot path)
}
```

`NodeData::Element` は `Box` で wrap: enum size を抑えるためと、Element
のみが持つ (~5 pointer size 分) を Text / Document node の memory footprint
に載せないため。blitz と同じ選択。

### 3.2 accessor methods

M1.15 で raikiri-dom 内部の 1 テスト
(`crates/raikiri-dom/src/lib.rs:639-654`
`node_pub_fields_are_readable_from_external_call_site`) が `node.kind` /
`node.tag_name` / `node.text_layout` を field 経由で参照している。この
refactor でその field access は壊れる。同名の accessor method を提供し、
呼び出し側の delta を `node.tag_name` → `node.tag_name()` の syntax
変更のみに抑える:

```rust
impl Node {
    #[inline]
    pub fn kind(&self) -> NodeKind {
        match &self.data {
            NodeData::Element(_) => NodeKind::Element,
            NodeData::Text(_) => NodeKind::Text,
            NodeData::Document => NodeKind::Document,
        }
    }

    #[inline]
    pub fn tag_name(&self) -> Option<&str> {
        match &self.data {
            NodeData::Element(e) => Some(e.tag_name.as_str()),
            _ => None,
        }
    }

    #[inline]
    pub fn text_layout(&self) -> Option<&parley::Layout<()>> {
        match &self.data {
            NodeData::Text(t) => t.text_layout.as_ref(),
            _ => None,
        }
    }

    #[inline]
    pub fn is_display_none(&self) -> bool { /* 現状維持 */ }

    #[inline]
    pub fn is_in_document(&self) -> bool {
        self.flags.contains(NodeFlags::IS_IN_DOCUMENT)
    }

    #[inline]
    pub(crate) fn set_in_document(&mut self, v: bool) {
        self.flags.set(NodeFlags::IS_IN_DOCUMENT, v);
    }
}
```

text_layout の mutation (`raikiri-dom/src/layout.rs:179` `node.text_layout = None`)
は data enum 経由に書き換える:
```rust
if let NodeData::Text(t) = &mut node.data {
    t.text_layout = None;
}
```

### 3.3 M1.15 契約への影響 (advisor 反映)

`crates/raikiri/tests/external_consumer.rs` (346 行) を熟読した結果、
**外部 consumer test は Node / Element の concrete field access を 1 件も
行っていない**。全て `#[non_exhaustive]` struct の Default / new / builder /
mutation 契約、`parse_html → plan → render_streaming` chain、および
`PageBox` / `PageDefaults` の値 pin。

concrete Node field pin は raikiri-dom 内部の 1 テストのみ
(`crates/raikiri-dom/src/lib.rs:639-654`)。この test を accessor 経由に
更新する:

```rust
#[test]
fn node_accessors_are_callable_from_external_call_site() {
    // Section 3.2 で導入した accessor が super::* から見えることを regression pin。
    // M1.15 の "pub Node field" pin を "pub Node accessor" pin に更新
    // (37c refactor 契約)。
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

外部契約は無影響、raikiri-dom 内部の pub_surface pin だけを accessor 経由
に再 pin する形。

## 4. NodeFlags

### 4.1 bitflags dep

`bitflags = "2"` を raikiri-dom の direct dep に追加。Cargo.lock で
`bitflags 2.13.0` が transitive にすでに存在するので追加コスト零。

### 4.2 型定義

```rust
// crates/raikiri-dom/src/node.rs

bitflags::bitflags! {
    /// Node per-node boolean 属性。blitz `NodeFlags` と bit 位置 1:1 対応
    /// (backport 前提)。
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct NodeFlags: u32 {
        /// この Node が flat tree に含まれるか。<template> 子孫は clear、
        /// Document root から flat-tree-parent 経由で到達可能なら set。
        ///
        /// 将来 shadow DOM / slot 対応時、slot 割当てられない host 直下 or
        /// shadow root 外の light-DOM 子孫も同 bit で表現する予定。
        const IS_IN_DOCUMENT = 1 << 0;
    }
}
```

将来の予約:
- `IS_INLINE_ROOT = 1 << 1` — inline formatting context root (M3)
- `IS_TABLE_ROOT = 1 << 2` — table formatting context root (M3+)

blitz と bit 位置を揃える (M6 blitz-compat crate が `NodeFlags::from_bits`
経由で機械変換可能)。

### 4.3 trait 拡張 (raikiri-traits::Node)

```rust
// crates/raikiri-traits/src/dom.rs

pub trait Node<'a> {
    type Element<'b>: Element<'b> where Self: 'b;

    fn kind(&self) -> NodeKind;
    fn as_element(&self) -> Option<Self::Element<'_>>;
    fn text_content(&self) -> Option<&str>;

    /// この Node が flat tree に含まれるか。<template> 子孫は `false`、
    /// Document root から flat-tree-parent 経由で到達可能な node は `true`。
    ///
    /// Traversal 側 (cascade / paint / stylesheet extract) はこの predicate
    /// で inert subtree を統一的に skip する。
    ///
    /// **Default impl は `true`** で、概念未対応の Node impl (test 用 stub 等)
    /// が silent drop されないための safe fallback (blitz `stylo.rs` の
    /// `TElement::is_in_document -> true` と同じ fallback 姿勢)。
    fn is_in_document(&self) -> bool {
        true
    }
}
```

raikiri-dom 側の `NodeRef` impl は override して実 bit を返す:
```rust
impl<'a> raikiri_traits::Node<'a> for NodeRef<'a> {
    /* ... existing kind / as_element / text_content */

    fn is_in_document(&self) -> bool {
        self.doc.nodes[self.id].is_in_document()
    }
}
```

## 5. sink.finish() phase 変更

### 5.1 phase 順序

```
Before:                          After:
  wire_side_tables               wire_side_tables
  extract_inline_stylesheets     mark_in_document_flags     ← NEW
  strip_non_element_stubs        extract_inline_stylesheets ← predicate 化
                                 strip_non_element_stubs
```

`mark_in_document_flags` は wire_side_tables **後** (namespace が populate
されないと `<template>` を HTML namespace で正確判定できない)、
extract_inline_stylesheets の **前**。

### 5.2 `mark_in_document_flags` 実装

```rust
fn mark_in_document_flags(doc: &mut Document) {
    // DFS from root、「in template subtree」boolean を stack に持ち回る。
    // <template> 判定は HTML namespace + local == "template" (case-sensitive、
    // html5ever は local を lowercase 済で提供する契約に依存)。
    // Iterative Vec stack で深い DOM の stack overflow を回避 (既存 walker と一貫)。
    let root = doc.root_index();
    let mut stack: Vec<(usize, bool /* in_template_subtree */)> = vec![(root, false)];
    while let Some((id, in_template)) = stack.pop() {
        let (children_snapshot, is_template_here) = {
            let node = &mut doc.nodes[id];
            node.set_in_document(!in_template);
            let is_template = matches!(&node.data, NodeData::Element(e)
                if e.tag_name.as_str() == "template" && e.namespace.is_none());
            (node.children.clone(), is_template)
        };
        let child_in_template = in_template || is_template_here;
        for c in children_snapshot.into_iter().rev() {
            stack.push((c, child_in_template));
        }
    }
}
```

**契約**:
- template 自身の bit は `!in_template` = true (template element も flat tree の一員)
- template 直下以降の子孫は `in_template = true` で降りるため bit clear
- HTML namespace 判定: `namespace.is_none()` (HTML default は None の fast path)
- SVG hypothetical `<template>` は namespace が Some なので **skip 対象外**
  (現状 ruletree.rs の case-insensitive 判定が持っていた false hit を解消)

### 5.3 `extract_inline_stylesheets` predicate 化

```rust
fn extract_inline_stylesheets(doc: &Document) -> Vec<String> {
    let Some(head_id) = find_head_element(doc) else { return Vec::new() };
    let mut out = Vec::new();
    let mut stack: Vec<NodeId> = vec![head_id];
    while let Some(id) = stack.pop() {
        if let Some(node) = doc.node(id) {
            if !node.is_in_document() {
                continue;  // <template> 子孫 + 将来の inert subtree を統一 skip
            }
            if let Some(el) = node.as_element() && el.tag_name() == "style" {
                let mut buf = String::new();
                for c in doc.child_ids(id) {
                    if let Some(child) = doc.node(c)
                        && let Some(t) = child.text_content()
                    {
                        buf.push_str(t);
                    }
                }
                if !buf.is_empty() { out.push(buf); }
                continue;
            }
        }
        let kids: Vec<_> = doc.child_ids(id).collect();
        for c in kids.into_iter().rev() { stack.push(c); }
    }
    out
}
```

現状の `"template" => continue` arm は削除。predicate で subsumed。

### 5.4 `get_template_contents` — 変更なし

```rust
fn get_template_contents(&self, target: &usize) -> usize {
    // M1 では template contents = template element 自身。真の fragment 分離は
    // M2+ (raikiri-spike-xno)。Section 3.1 で ElementData.template_contents
    // slot を予約したが populate はしていない (blitz と同じ TODO 状態)。
    *target
}
```

## 6. Traversal caller 統一

### 6.1 `raikiri-style::ruletree::walk_and_collect`

```rust
while let Some(id) = stack.pop() {
    if let Some(node) = dom.node(id) {
        if !node.is_in_document() { continue; }  // template 子孫を skip
        if let Some(elem) = node.as_element()
            && elem.tag_name().eq_ignore_ascii_case("style")
        { /* 収集 */ }
        // children push
    }
}
```

`"template"` string 比較を削除。predicate に集約。

### 6.2 `raikiri-style::cascade::collect_cascaded` — silent bug fix

```rust
while let Some(id) = stack.pop() {
    if let Some(node) = dom.node(id) {
        if !node.is_in_document() { continue; }  // NEW: cascade も skip
        // ... 既存の element 判定と rule matching
    }
}
```

`resolve_inheritance` の iterative walk にも同じ gate を入れる。

### 6.3 `raikiri-paint::walk::paint_document` — silent bug fix

```rust
while let Some((node_id, parent_abs_x, parent_abs_y)) = stack.pop() {
    let Some(node) = document.get_node(node_id) else { continue };
    if !node.is_in_document() { continue; }  // NEW: display:none 依存を解除
    match node.kind() {
        NodeKind::Element => {
            if node.is_display_none() { continue; }
            /* ... 既存 */
        }
        /* ... */
    }
}
```

UA CSS の `template { display: none }` rule 有無に依存しない、明示的な skip。
確認済: 現状の UA CSS (`crates/raikiri-html/src/ua/minimal.css`) には
template rule 無し、silent bug 表面が実存。

### 6.4 変更影響ファイル一覧

| ファイル | 変更 |
|---|---|
| `raikiri-dom/Cargo.toml` | `bitflags = "2"` direct dep |
| `raikiri-dom/src/node.rs` | NodeData / ElementData / TextData / NodeFlags 導入、Node 再構成、accessor methods |
| `raikiri-dom/src/document.rs` | append_element / append_text / set_element_* が NodeData 経由へ (crate 内)、`set_in_document` crate-private setter |
| `raikiri-dom/src/dom_impl.rs` | NodeRef / ElementRef が NodeData 経由 field 参照へ、`is_in_document()` trait impl override |
| `raikiri-dom/src/layout.rs` | find_body / preshape_text が accessor 経由へ |
| `raikiri-dom/src/lib.rs` | 内部 pub_surface pin test を accessor pin へ改名 (§3.3) |
| `raikiri-html/src/sink.rs` | `mark_in_document_flags` 追加、phase 順序変更、`extract_inline_stylesheets` predicate 化、`get_template_contents` コメント更新 |
| `raikiri-html/src/lib.rs` | 新 regression fixture 4 本 (§7) |
| `raikiri-style/src/ruletree.rs` | `walk_and_collect` に predicate gate |
| `raikiri-style/src/cascade.rs` | `collect_cascaded` / `resolve_inheritance` に predicate gate |
| `raikiri-paint/src/walk.rs` | `paint_document` に predicate gate + accessor 経由化 |
| `raikiri-paint/src/text.rs` | accessor 経由化 |
| `raikiri-paint/src/lib.rs` | 新 regression fixture 1 本 (§7) |
| `raikiri-traits/src/dom.rs` | `Node<'a>` trait に `is_in_document() -> bool { true }` default 追加 |

## 7. テスト戦略 (advisor #2 反映)

### 7.1 テスト配置

TestDoc (raikiri-style 内 stub Dom impl) は `is_in_document()` default true
を継承するため、cascade / paint の "template 内 element を skip する"
regression を TestDoc で書けない。整合的な選択は **integration test 側に
配置** — parse → cascade / parse → paint を通して bit populate 経由で検証する。

副次的に、これらは真の pipeline regression になり、単体 unit test より
価値が高い。

| テスト | 配置 crate | 目的 |
|---|---|---|
| `parse_marks_body_children_in_document` | raikiri-html | 正常経路の bit populate |
| `parse_marks_template_descendants_out_of_document` | raikiri-html | template 直下の bit clear、template 自身は set |
| `parse_marks_nested_template_descendants_out_of_document` | raikiri-html | 深いネストの bit 伝播 |
| `parse_skips_style_inside_template_element` | raikiri-html | 既存継続 (predicate 経由でも pass) |
| `parse_then_cascade_skips_template_descendants` | raikiri-html | Section 6.2 の silent bug 回帰 pin |
| `parse_then_paint_skips_template_subtree` | raikiri-paint | Section 6.3 の silent bug 回帰 pin |
| `node_accessors_are_callable_from_external_call_site` | raikiri-dom | §3.3 renamed pub_surface pin |
| `is_in_document` doctest | raikiri-traits | default true の semantics 説明 |

### 7.2 Acceptance criteria

新 issue (raikiri-spike-37c) の close 条件:

1. `<template>` 関連 traversal 全 5 か所 (extract / walk_and_collect /
   collect_cascaded / resolve_inheritance / paint_document) が
   `is_in_document()` predicate で統一 skip
2. ad-hoc string 判定 `"template"` は **sink の `mark_in_document_flags`
   判定 site 1 箇所のみ** に集約 (grep で回帰確認可能)
3. §7.1 regression fixture 6 件が pass
4. raikiri-dom internal pub_surface pin が accessor 経由で通る
5. M1.15 external consumer test は無変更で pass
6. raikiri-spike-xno issue 本文を Part 2 (真の fragment separation) に scope
   縮小 (open のまま残す)

### 7.3 lint / build 検証

- `cargo build --workspace` — bitflags dep 追加後 pass
- `cargo test --workspace` — 全 test pass + 新規 6 件 pass
- `cargo doc --workspace --no-deps` — missing_docs warn 通過 (新 API 全てに doc comment)
- `cargo clippy --workspace -- -D warnings` — dead_code / template_contents field は
  `#[derive(Debug)]` の derived read で silence (advisor #3 反映) + doc comment で
  意図明文化

## 8. Defer (この design の対象外)

| 項目 | 状態 | 将来 |
|---|---|---|
| `get_template_contents` の真の fragment 返却 | `*target` 継続 | M2+、raikiri-spike-xno で対応 (Part 2) |
| Runtime mutation の bit 維持 | 発生しない (parse-only) | M2+ で blitz `process_added_subtree` 相当を mutator に追加 |
| Shadow DOM / slot semantics | 概念未導入 | 新 issue、`IS_IN_DOCUMENT` bit slot を再利用、set/clear ロジック追加 |
| `IS_INLINE_ROOT` / `IS_TABLE_ROOT` | NodeFlags に定義せず | M3 で inline / table formatting root 実装時、blitz と同 bit 位置で追加 |
| MathML integration point との相互作用 | 直交、変更なし | — |
| Foster parenting 中の transient node の bit | 発生しない (sink.finish 一発 pass) | mutator 導入時 |

## 9. Implementation phase 順序

```
Phase 1: raikiri-dom refactor
  1. bitflags dep 追加
  2. NodeFlags 型導入
  3. NodeData enum + ElementData + TextData 導入
  4. Node struct 再構成 + accessor methods
  5. Document::append_element / append_text / set_element_* を新 shape へ
  6. dom_impl.rs NodeRef / ElementRef を新 shape へ
  7. layout.rs / dom_impl.rs 内部 field 参照を accessor へ
  8. 内部 pub_surface pin test を accessor 経由に改名

Phase 2: raikiri-traits Node trait 拡張
  1. is_in_document default true 追加 + doctest

Phase 3: raikiri-html sink
  1. mark_in_document_flags 実装
  2. finish() の phase 順序変更
  3. extract_inline_stylesheets を predicate 経由へ
  4. 新 regression 4 件 (parse_marks_* + parse_then_cascade)
  5. get_template_contents コメント更新 (populate は依然 defer)

Phase 4: raikiri-style traversal
  1. walk_and_collect predicate 化
  2. collect_cascaded / resolve_inheritance に gate 追加

Phase 5: raikiri-paint
  1. paint_document predicate gate + accessor 経由化
  2. text.rs accessor 経由化
  3. parse_then_paint_skips_template_subtree fixture

Phase 6: docs + issue update
  1. raikiri-dom module doc に flat tree section 追記
  2. raikiri-spike-xno 本文を Part 2 scope に更新
```

## Appendix A. Blitz 現状の抜粋 (2026-07-18 時点)

`/home/mitz/Work/oss/blitz/packages/blitz-dom/src/node/node.rs`:
```rust
bitflags::bitflags! {
    #[derive(Clone, Copy, PartialEq)]
    pub struct NodeFlags: u32 {
        const IS_INLINE_ROOT = 0b00000001;
        const IS_TABLE_ROOT = 0b00000010;
        const IS_IN_DOCUMENT = 0b00000100;
    }
}
```

`/home/mitz/Work/oss/blitz/packages/blitz-dom/src/node/element.rs`:
```rust
/// The element's template contents (\<template\> elements only)
pub template_contents: Option<usize>,
```

`/home/mitz/Work/oss/blitz/packages/blitz-html/src/html_sink.rs`:
```rust
fn get_template_contents(&self, target: &Self::Handle) -> Self::Handle {
    // TODO: implement templates properly. This should allow to function like regular elements.
    *target
}
```

`/home/mitz/Work/oss/blitz/packages/blitz-dom/src/mutator.rs:643`:
```rust
fn process_added_subtree(&mut self, node_id: usize) {
    self.doc.iter_subtree_mut(node_id, |node_id, doc| {
        let node = &mut doc.nodes[node_id];
        node.flags.set(NodeFlags::IS_IN_DOCUMENT, true);
        // ...
    });
}
```

raikiri は (i) `IS_IN_DOCUMENT` bit の維持を parse-only なので sink.finish 一発
DFS で担う、(ii) `template_contents` slot だけ予約、という段階的アプローチ
で blitz に nominal に揃える。
