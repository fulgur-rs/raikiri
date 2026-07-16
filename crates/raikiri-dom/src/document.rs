//! Document arena — Vec-backed node arena that implements taffy layout traits
//! (in `taffy_impl.rs`) and raikiri-traits::Dom (in `dom_impl.rs`).

use smol_str::SmolStr;
use std::borrow::Cow;
use taffy::Style;

use raikiri_traits::{NodeKind, StylesheetKind};

use crate::node::{Attr, Node};

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
    /// Panics (debug builds only): `id` が Element kind でない場合。Text /
    /// Document node に attribute-family setter を呼ぶのは caller bug なので
    /// early fail させる。
    pub fn set_element_namespace(&mut self, id: usize, ns: Option<SmolStr>) {
        debug_assert_eq!(
            self.nodes[id].kind,
            NodeKind::Element,
            "set_element_namespace called on non-Element (id={id})"
        );
        self.nodes[id].namespace = ns;
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
    /// Panics (debug builds only): `id` が Element kind でない場合。
    pub fn set_element_attributes(&mut self, id: usize, attrs: Vec<(SmolStr, SmolStr)>) {
        debug_assert_eq!(
            self.nodes[id].kind,
            NodeKind::Element,
            "set_element_attributes called on non-Element (id={id})"
        );
        self.nodes[id].attributes = attrs
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
    /// Panics (debug builds only): `id` が Element kind でない場合。
    pub fn set_element_inline_style(&mut self, id: usize, inline_style: Option<SmolStr>) {
        debug_assert_eq!(
            self.nodes[id].kind,
            NodeKind::Element,
            "set_element_inline_style called on non-Element (id={id})"
        );
        self.nodes[id].inline_style = inline_style;
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
