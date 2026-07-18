//! Document arena — Vec-backed node arena that implements taffy layout traits
//! (in `taffy_impl.rs`) and raikiri-traits::Dom (in `dom_impl.rs`).

use smol_str::SmolStr;
use std::borrow::Cow;
use taffy::Style;

use raikiri_traits::StylesheetKind;

use crate::node::{Attr, Node, NodeData};

/// DOM Document (root + Vec-backed node arena)。
///
/// `nodes` は arena indices を key とする flat storage。index 0 は Document
/// kind の virtual root。HTML の `<html>` element は M1.3 html-parse-basic が
/// index 1 以降に append する想定 (root = 0 の子として)。
#[derive(Debug)]
pub struct Document {
    pub(crate) nodes: Vec<Node>,
    /// arena index of the Document root (always 0 の予定、明示的に保持して
    /// 将来 detach root 等の変則 case に備える)。
    pub(crate) root: usize,
    /// Layout cache dirty flag。任意の tree mutation で set され、次の
    /// `compute_child_layout` の頭で lazy に全 node cache clear + reset
    /// する。O(1) per-mutation cost + O(N) per-layout-batch cost で
    /// invalidation の amortized O(1) を実現。
    pub(crate) layout_dirty: bool,
    /// Document に associate されている stylesheet の list (M1.4a、
    /// raikiri-spike-m1.22)。lazy: parse は cascade phase で行う。
    /// 呼び出し順で同 kind 内の cascade source_order が決まる。
    stylesheets: Vec<(Cow<'static, str>, StylesheetKind)>,
}

impl Document {
    /// 新しい Document を構築する。arena index 0 に Document kind node を配置。
    pub fn new() -> Self {
        let mut nodes = Vec::with_capacity(16);
        nodes.push(Node::new_document());
        Self {
            nodes,
            root: 0,
            layout_dirty: false,
            stylesheets: Vec::new(),
        }
    }

    /// Element node を arena に追加する。`parent` が `Some(idx)` の場合
    /// その node の children に append される。`None` の場合 detached (どこにも
    /// 属さない fragment、後で attach する用途)。
    ///
    /// `inline_style` は HTML `style="..."` attribute の生 string を渡す
    /// (`None` = 属性なし)。raikiri-style::cascade (M1.4) が消費する。
    ///
    /// Returns: 追加された node の arena index。
    pub fn append_element(
        &mut self,
        parent: Option<usize>,
        tag: impl Into<SmolStr>,
        style: Style,
        inline_style: Option<impl Into<SmolStr>>,
    ) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node::new_element(
            tag.into(),
            style,
            inline_style.map(Into::into),
        ));
        if let Some(p) = parent {
            self.nodes[p].children.push(id);
        }
        self.invalidate_layout_cache();
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
        self.invalidate_layout_cache();
        id
    }

    /// 既存の detached node を `parent` の末尾 child として attach する。
    ///
    /// html5ever `TreeSink::append(parent, AppendNode(child))` の primitive。
    /// `child` は既に arena に存在している必要があり、既に別 parent の下にいる
    /// 場合は事前に [`Document::detach_from_parent`] で detach しておくこと
    /// (tree の重複配置を防ぐため raikiri-dom は自動 detach しない)。
    pub fn attach_child(&mut self, parent: usize, child: usize) {
        self.nodes[parent].children.push(child);
        self.invalidate_layout_cache();
    }

    /// `parent` の children 配列内、`before` の直前 index に `child` を挿入する。
    ///
    /// html5ever `TreeSink::append_before_sibling(sibling, AppendNode(child))`
    /// および foster parenting の primitive。`before` が `parent` の子でない場合
    /// は末尾に append する (defensive: TreeSink 規約上発生しない想定)。
    pub fn insert_child_before(&mut self, parent: usize, before: usize, child: usize) {
        let kids = &mut self.nodes[parent].children;
        if let Some(pos) = kids.iter().position(|&c| c == before) {
            kids.insert(pos, child);
        } else {
            // html5ever TreeSink contract 上ここには来ない。dev/test では contract
            // 違反として panic、release では plan 指定の tail-append fallback。
            debug_assert!(
                false,
                "insert_child_before: `before` ({before}) not a child of parent ({parent})"
            );
            kids.push(child);
        }
        self.invalidate_layout_cache();
    }

    /// `child` を保持する parent の arena index を返す。root (index 0) や
    /// 未 attach node は `None`。linear scan (O(N))、TreeSink の呼び出し
    /// 頻度は多くないため raikiri-dom は parent pointer 非保持。
    pub fn parent_of(&self, child: usize) -> Option<usize> {
        self.nodes
            .iter()
            .enumerate()
            .find_map(|(i, n)| n.children.contains(&child).then_some(i))
    }

    /// `child` を現在の parent から取り除く。除去した親 index を返す。
    /// 未 attach の場合は `None` (no-op)。
    ///
    /// html5ever `TreeSink::remove_from_parent(target)` の primitive。
    pub fn detach_from_parent(&mut self, child: usize) -> Option<usize> {
        let parent = self.parent_of(child)?;
        let kids = &mut self.nodes[parent].children;
        if let Some(pos) = kids.iter().position(|&c| c == child) {
            kids.remove(pos);
        }
        self.invalidate_layout_cache();
        Some(parent)
    }

    /// `from` の全 children を `to` の children 末尾に move する。`from`
    /// の children は空になる。html5ever `TreeSink::reparent_children` の primitive。
    pub fn reparent_children(&mut self, from: usize, to: usize) {
        let moved: Vec<usize> = self.nodes[from].children.drain(..).collect();
        self.nodes[to].children.extend(moved);
        self.invalidate_layout_cache();
    }

    /// Element node に non-HTML namespace URI を紐付ける
    /// (raikiri-spike-blg)。`ns` が `None` = HTML default namespace / element
    /// でない場合の効果無し。HTML default は `None` を fast path とする
    /// (memory saving + `Element::namespace_uri()` の O(1) 判定)。
    ///
    /// raikiri-html sink が `finish()` 時に qual_names side-table から呼び出す。
    /// tree mutation ではないので `invalidate_layout_cache` は call しない。
    ///
    /// Panics (debug + release 共通、raikiri-spike-37c): `id` が Element kind
    /// でない場合。Text / Document node に attribute-family setter を呼ぶのは
    /// caller bug なので early fail させる (旧 `debug_assert_eq!` から
    /// `NodeData::as_element_mut().expect(...)` に移行、release でも panic する
    /// ようになったのは意図的な strictness 向上)。
    pub fn set_element_namespace(&mut self, id: usize, ns: Option<SmolStr>) {
        let e = self.nodes[id]
            .data
            .as_element_mut()
            .expect("set_element_namespace called on non-Element");
        e.namespace = ns;
    }

    /// Element node に attribute list を紐付ける (raikiri-spike-blg)。
    /// `attrs` は null-namespace attribute の `(local, value)` 列。html5ever の
    /// source order を保持する必要があるので Vec で受ける。`style` attribute は
    /// [`Document::set_element_inline_style`] で別途 wire するため呼び出し側で
    /// 除外しておくこと。
    ///
    /// raikiri-html sink が `finish()` 時に attributes side-table から呼び出す。
    /// tree mutation ではないので `invalidate_layout_cache` は call しない。
    ///
    /// Panics (debug + release 共通、raikiri-spike-37c): `id` が Element kind
    /// でない場合。
    pub fn set_element_attributes(&mut self, id: usize, attrs: Vec<(SmolStr, SmolStr)>) {
        let e = self.nodes[id]
            .data
            .as_element_mut()
            .expect("set_element_attributes called on non-Element");
        e.attributes = attrs
            .into_iter()
            .map(|(local, value)| Attr { local, value })
            .collect();
    }

    /// Element node の `inline_style` を後付けで更新する
    /// (raikiri-spike-blg)。sink が `finish()` 時に side-table から
    /// `style="..."` を抽出して呼び出す。値は生 string でよく、`style=""`
    /// の空文字列 → `None` 正規化は Element trait 実装側
    /// ([`raikiri_traits::Element::inline_style_source`]) が行う。
    /// 二重正規化を避けるため storage 層はここで判定しない。
    ///
    /// Panics (debug + release 共通、raikiri-spike-37c): `id` が Element kind
    /// でない場合。
    pub fn set_element_inline_style(&mut self, id: usize, inline_style: Option<SmolStr>) {
        let e = self.nodes[id]
            .data
            .as_element_mut()
            .expect("set_element_inline_style called on non-Element");
        e.inline_style = inline_style;
    }

    /// 全 node の children Vec に対して predicate を適用し、`false` を返す
    /// entry を除去する。html5ever の comment / PI stub を single-pass で
    /// 除去する目的で raikiri-html が使用する。個別に `detach_from_parent`
    /// を呼ぶ O(K*N) 実装を回避 (attacker-controlled な多量 stub で quadratic
    /// を防ぐ)。tree mutation なので `invalidate_layout_cache` も call する。
    pub fn retain_children(&mut self, mut predicate: impl FnMut(usize) -> bool) {
        let mut any_removed = false;
        for node in &mut self.nodes {
            let before = node.children.len();
            node.children.retain(|&c| predicate(c));
            if node.children.len() != before {
                any_removed = true;
            }
        }
        if any_removed {
            self.invalidate_layout_cache();
        }
    }

    /// `<template>` element の子孫について `IS_IN_DOCUMENT` bit を clear する
    /// (raikiri-spike-37c)。sink.finish() から呼ばれる。
    ///
    /// アルゴリズム (roborev job 292 findings 対応):
    /// 1. 全 arena node の bit を先に clear (detached / unreachable node を
    ///    default true のまま残さないため)
    /// 2. Document root から iterative DFS で bit set。template element 自身
    ///    は set、その descendants は skip (bit clear の状態が残る)
    ///
    /// 実装上の細かい contract:
    /// - `<template>` 判定は HTML namespace + local == "template"
    ///   (case-sensitive)。html5ever が local を lowercase 済で提供する契約に
    ///   依存。SVG hypothetical `<template>` (別 namespace) は skip 対象外
    ///   (spec-correct: SVG に `<template>` はそもそも定義が無いが raw parser で
    ///   混入し得るため defensive)。
    /// - foster parenting 中の transient detached node は Vec::retain 系
    ///   mutation (`Document::retain_children`) や `detach_from_parent` で
    ///   arena の子 pointer だけ切れた state になり得る。本 method は step 1 で
    ///   全 node を clear するため、そうした node が in_document=true として
    ///   残ることは無い。
    /// - iterative Vec stack で深い DOM での stack overflow を回避。
    /// - roborev job 293 M2 finding: 本 method は taffy の effective child tree
    ///   (`TaffyChildIter` が `is_in_document()` で filter する) を変更する。
    ///   post-condition として layout cache も無効化する — さもなくば次回
    ///   `compute_child_layout` が古い child ordering で cached result を再利用
    ///   してしまう。
    pub fn mark_in_document_flags(&mut self) {
        // Step 1: 全 arena node の bit を先に clear。Node::new_* constructor が
        // default true を立てるが、それは "attach 済み" の楽観的初期値。ここで
        // 明示的に clear することで detached / unreachable node が false に落ちる。
        for node in &mut self.nodes {
            node.set_in_document(false);
        }
        // Step 2: Document root から reachable な node を DFS で set。
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
        // Step 3: taffy が観測する effective child tree が変わり得るため、
        // layout cache を dirty mark する (roborev job 293 M2 finding)。
        self.invalidate_layout_cache();
    }

    /// arena index `id` の node への借用参照。範囲外 index は `None`。
    ///
    /// blitz-dom の `BaseDocument::get_node` 相当。raikiri-paint が walk 中に
    /// per-node で呼ぶ hot path なので O(1) の `Vec::get` を wrap。
    pub fn get_node(&self, id: usize) -> Option<&Node> {
        self.nodes.get(id)
    }

    /// arena 内の総 node 数 (Document root を含む)。
    ///
    /// raikiri-paint / caller が `cascade.computed.len() == doc.node_count()`
    /// の contract violation を early に検出する目的 + doctest / smoke test で消費。
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Document root の arena index (常に 0)。
    ///
    /// `Dom::root_id()` trait method の inherent 版。trait import せず
    /// `&Document` から直接呼べる。
    /// 命名: `root_index` (type は `usize` で trait method の `NodeId` newtype と区別)
    pub fn root_index(&self) -> usize {
        self.root
    }

    /// tree mutation を layout cache dirty として mark する。実際の cache
    /// clear は次回 `compute_child_layout` (taffy_impl 経由) で lazy に発火する。
    /// per-mutation は O(1)、per-layout-batch で amortized O(N)。
    fn invalidate_layout_cache(&mut self) {
        self.layout_dirty = true;
    }

    // ─── stylesheets (M1.4a、raikiri-spike-m1.22) ───────────────────

    /// Stylesheet を Document に associate する。
    ///
    /// - lazy: parse は行わない。cascade phase で一括処理される。
    /// - 呼び出し順で同一 `kind` 内の cascade source_order が決まる
    ///   (spec §M1.4a)。
    /// - `Cow<'static, str>` により、bundled UA CSS 等 static &str は
    ///   borrow のまま保持され allocation なし。Consumer 提供の
    ///   `String` は Cow::Owned で消費される。
    pub fn add_stylesheet(&mut self, source: impl Into<Cow<'static, str>>, kind: StylesheetKind) {
        self.stylesheets.push((source.into(), kind));
    }

    /// 現在 associate されている全 stylesheet を `(source, kind)` の
    /// tuple として iterate する。順序は `add_stylesheet` の呼び出し順。
    ///
    /// cascade orchestrator (raikiri umbrella) が RuleTree 構築時に
    /// consume する想定。
    pub fn stylesheets(&self) -> impl Iterator<Item = (&str, StylesheetKind)> + '_ {
        self.stylesheets
            .iter()
            .map(|(cow, kind)| (cow.as_ref(), *kind))
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod stylesheets_tests {
    use super::*;
    use raikiri_traits::StylesheetKind;
    use std::borrow::Cow;

    #[test]
    fn document_add_stylesheet_appends_in_call_order() {
        let mut doc = Document::new();
        doc.add_stylesheet(Cow::Borrowed("a { color: red }"), StylesheetKind::UserAgent);
        doc.add_stylesheet(
            Cow::Owned("b { color: blue }".to_string()),
            StylesheetKind::Author,
        );

        let collected: Vec<(&str, StylesheetKind)> = doc.stylesheets().collect();
        assert_eq!(collected.len(), 2);
        assert_eq!(
            collected[0],
            ("a { color: red }", StylesheetKind::UserAgent)
        );
        assert_eq!(collected[1], ("b { color: blue }", StylesheetKind::Author));
    }

    #[test]
    fn document_add_stylesheet_borrow_variant_zero_alloc() {
        // Cow::Borrowed を渡した場合、内部 storage も Borrowed のまま保持される
        // ことを as_ref() 経由で確認する (pointer 比較で same 'static addr)。
        let mut doc = Document::new();
        let ua: &'static str = "html { display: block }";
        doc.add_stylesheet(Cow::Borrowed(ua), StylesheetKind::UserAgent);
        let (source, _kind) = doc.stylesheets().next().expect("has one");
        assert!(
            std::ptr::eq(source, ua),
            "borrowed source should keep &'static identity"
        );
    }

    #[test]
    fn document_stylesheets_iterates_mixed_kinds() {
        let mut doc = Document::new();
        doc.add_stylesheet(Cow::Borrowed("ua1"), StylesheetKind::UserAgent);
        doc.add_stylesheet(Cow::Borrowed("au1"), StylesheetKind::Author);
        doc.add_stylesheet(Cow::Borrowed("ua2"), StylesheetKind::UserAgent);

        let kinds: Vec<StylesheetKind> = doc.stylesheets().map(|(_, k)| k).collect();
        assert_eq!(
            kinds,
            vec![
                StylesheetKind::UserAgent,
                StylesheetKind::Author,
                StylesheetKind::UserAgent,
            ]
        );
    }
}

#[cfg(test)]
mod mark_in_document_flags_tests {
    use super::*;

    #[test]
    fn mark_in_document_flags_clears_detached_arena_nodes() {
        // raikiri-spike-37c roborev job 292 M2 finding pin。arena に存在するが
        // Document root から reachable でない node (foster-parenting transient
        // state / stub 除去後の孤児 等) は mark 後 is_in_document=false に落ちる
        // (Node::new_* の default true を step 1 の全 clear が上書きする)。
        let mut doc = Document::new();
        let root = doc.root_index();
        let attached = doc.append_element(Some(root), "div", Style::default(), None::<&str>);
        // parent=None で detached を作る (append_element の primitive contract)
        let detached = doc.append_element(None::<usize>, "span", Style::default(), None::<&str>);

        // constructor default はどちらも true
        assert!(doc.get_node(attached).unwrap().is_in_document());
        assert!(doc.get_node(detached).unwrap().is_in_document());

        doc.mark_in_document_flags();

        assert!(
            doc.get_node(attached).unwrap().is_in_document(),
            "attached div should remain in_document after mark"
        );
        assert!(
            !doc.get_node(detached).unwrap().is_in_document(),
            "detached span must be cleared to !in_document after mark"
        );
    }

    #[test]
    fn mark_in_document_flags_keeps_template_element_but_clears_descendants() {
        // Regression pin for the existing contract Task 3 pinned: template element
        // itself stays in_document=true, its descendants get cleared. Redundant
        // with the raikiri-html integration test but locally verifies the DFS
        // shape (in_template state propagation) without going through parse.
        let mut doc = Document::new();
        let root = doc.root_index();
        let tmpl = doc.append_element(Some(root), "template", Style::default(), None::<&str>);
        let inner = doc.append_element(Some(tmpl), "p", Style::default(), None::<&str>);
        let text = doc.append_text(inner, "hi");

        doc.mark_in_document_flags();

        assert!(doc.get_node(tmpl).unwrap().is_in_document(), "template stays in doc");
        assert!(!doc.get_node(inner).unwrap().is_in_document(), "<p> cleared");
        assert!(!doc.get_node(text).unwrap().is_in_document(), "text cleared");
    }
}

#[cfg(test)]
mod taffy_filter_tests {
    use super::*;
    use taffy::TraversePartialTree;

    #[test]
    fn taffy_child_ids_and_count_filter_out_template_descendants() {
        // raikiri-spike-37c roborev job 292 M1 finding pin。taffy layout tree
        // (= web spec flat tree) から template descendants を除外する。
        // template 自身は in_document=true なので body の child 数に含まれる、
        // その内側の <p> は in_document=false なので template の taffy child
        // 数 = 0 になる。
        let mut doc = Document::new();
        let root = doc.root_index();
        let body = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
        let tmpl = doc.append_element(Some(body), "template", Style::default(), None::<&str>);
        let inner = doc.append_element(Some(tmpl), "p", Style::default(), None::<&str>);
        let _txt = doc.append_text(inner, "hi");

        doc.mark_in_document_flags();

        let body_id = taffy::NodeId::from(body);
        let tmpl_id = taffy::NodeId::from(tmpl);

        // body の直接子は template 1 個 (taffy 経由 count)
        assert_eq!(
            <Document as TraversePartialTree>::child_count(&doc, body_id),
            1,
            "body has template as its one filtered child"
        );
        let body_children: Vec<taffy::NodeId> =
            <Document as TraversePartialTree>::child_ids(&doc, body_id).collect();
        assert_eq!(body_children, vec![tmpl_id]);

        // template の taffy view から見た child_count = 0 (inner <p> は filter される)
        assert_eq!(
            <Document as TraversePartialTree>::child_count(&doc, tmpl_id),
            0,
            "template contents are filtered out of taffy layout tree"
        );
        let tmpl_children: Vec<taffy::NodeId> =
            <Document as TraversePartialTree>::child_ids(&doc, tmpl_id).collect();
        assert!(tmpl_children.is_empty());
    }
}
