//! Document arena — Vec-backed node arena that implements taffy layout traits
//! (in `taffy_impl.rs`) and raikiri-traits::Dom (in `dom_impl.rs`).
//!
//! # Flat tree membership contract (raikiri-spike-37c)
//!
//! 全 tree mutation primitive (`append_*` / `attach_child` /
//! `insert_child_before` / `detach_from_parent` / `reparent_children` /
//! `retain_children` / `set_element_namespace` for template elements) は
//! [`Document::flags_dirty`] を `true` に set する。`Node::is_in_document()`
//! を観測する caller は observation 前に
//! [`Document::mark_in_document_flags`] を呼んで bit を re-sync する必要が
//! ある。`mark_in_document_flags` は `!flags_dirty` のとき O(1) の no-op
//! なので、多重呼び出しも安全。
//!
//! Auto-sync entry:
//! - `raikiri-html::sink::finish()` が parse の観測境界で呼ぶ
//! - `raikiri-dom::layout_single_page()` が layout/paint の観測境界で呼ぶ
//!
//! Manual-sync required:
//! - `raikiri-style::cascade()` は `&D: Dom` を取るため mutation 不可、
//!   sync 呼び出しを caller に委ねる。cascade を直接呼ぶ consumer は
//!   parse 経由でしか自動 sync されないため、post-parse mutation の後は
//!   明示的に `mark_in_document_flags()` する必要がある。

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
    /// IS_IN_DOCUMENT bit dirty flag (raikiri-spike-37c, roborev job 294 M1
    /// finding 対応)。任意の tree-mutation primitive (append_* / attach_child
    /// / insert_child_before / detach_from_parent / reparent_children /
    /// retain_children) で set される。observation-side API (cascade / paint /
    /// extract) は生の bit を信じる前に [`Document::mark_in_document_flags`]
    /// を呼ぶことで dirty check + lazy recompute を強制する contract。
    ///
    /// M1 parse-only では `sink.finish()` が明示的に呼ぶため無視できるが、
    /// 手動で `append_*` を呼んで Document を組み立てる code path (raikiri-dom
    /// 内 test / raikiri-paint hello-world setup / 将来の M2+ mutation runtime)
    /// では本 dirty flag が correctness の contract を担う。
    pub(crate) flags_dirty: bool,
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
            // 初期 root node は Node::new_document() が IS_IN_DOCUMENT=true を
            // 立てているため、"attached under root" として consistent。まだ
            // template も detached node も無いので dirty ではない。
            flags_dirty: false,
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
    /// # Contract (raikiri-spike-37c)
    ///
    /// 本 method は [`flags_dirty`](Self#structfield.flags_dirty) を `true` に
    /// set する。`Node::is_in_document()` を観測する caller (raikiri-style::cascade
    /// / raikiri-dom::layout_single_page / raikiri-paint::paint_single_page)
    /// は、mutation batch 後に [`mark_in_document_flags`](Self::mark_in_document_flags)
    /// を呼んで bit を re-sync する必要がある。`layout_single_page` は entry で
    /// 自動 sync するため、layout/paint pipeline のみを消費する consumer は
    /// 明示呼び出し不要。cascade を直接呼ぶ場合は明示 sync が必要。
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
        self.flags_dirty = true;
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
        self.flags_dirty = true;
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
        self.flags_dirty = true;
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
        self.flags_dirty = true;
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
        self.flags_dirty = true;
        Some(parent)
    }

    /// `from` の全 children を `to` の children 末尾に move する。`from`
    /// の children は空になる。html5ever `TreeSink::reparent_children` の primitive。
    pub fn reparent_children(&mut self, from: usize, to: usize) {
        let moved: Vec<usize> = self.nodes[from].children.drain(..).collect();
        self.nodes[to].children.extend(moved);
        self.invalidate_layout_cache();
        self.flags_dirty = true;
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
        // raikiri-spike-37c roborev job 295 M2 finding: namespace の変更は
        // `<template>` 判定 (`namespace.is_none()` は HTML default fast path) を
        // 変え得るため、tag_name が "template" の場合は flags_dirty を set する。
        // これがないと HTML template → SVG template への変更 (あるいは逆) の後
        // `mark_in_document_flags()` が early return path で no-op となり、
        // 子孫の in_document bit が stale のまま残る。
        //
        // template 以外の element では namespace 変更は本 bit に無関係なので
        // flag は set しない (invalidate_layout_cache も呼ばない: pure metadata
        // 変更で layout 結果を変えない、既存 blg 契約と一貫)。
        let ns_changed_for_template = {
            let e = self.nodes[id]
                .data
                .as_element_mut()
                .expect("set_element_namespace called on non-Element");
            let is_template_tag = e.tag_name.as_str() == "template";
            let changed = e.namespace != ns;
            e.namespace = ns;
            is_template_tag && changed
        };
        if ns_changed_for_template {
            self.flags_dirty = true;
        }
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

    /// `<template>` element の contents fragment root を新規 allocate し、
    /// その arena index を template element の `template_contents` slot に
    /// wire する (raikiri-spike-xno Part 2)。
    ///
    /// # Fragment root の shape (Option B — detached subtree)
    ///
    /// Fragment root は `Document.nodes` arena に detached 状態で allocate される
    /// (parent なし、Document root からも reachable でない)。tag は
    /// `"#document-fragment"` — 既存の `#comment` / `#pi` pseudo-tag convention
    /// を踏襲する:
    /// - CSS selector は leading `#` の tag 名にマッチしないため、意図しない
    ///   selector match / cascade が起きない
    /// - real HTML tag と衝突しない
    /// - `NodeKind::Element` として存在するが `is_in_document()` は false
    ///   (以下 `flags_dirty=true` → `mark_in_document_flags` の step 1 で clear
    ///   された後、step 2 で reachable でないため false のまま)
    ///
    /// # html5ever integration
    ///
    /// html5ever `TreeSink::create_element` に渡される `ElementFlags::template`
    /// が true の時、sink がこの method を呼んで fragment root を作り
    /// template element の `template_contents` slot に格納する。以降
    /// `TreeSink::get_template_contents` は fragment root index を返し、
    /// html5ever は template contents をその子として append する
    /// (template element 自身の children は空のまま)。
    ///
    /// blitz `blitz-dom::html_sink::HtmlSink::create_element` の
    /// `create_template_contents` 相当。
    ///
    /// # Panics
    ///
    /// - `template_id` が Element kind でない場合 (release + debug 共通)。
    ///   template element でない node に fragment root を wire するのは
    ///   caller bug なので early fail。
    /// - `template_id` の Element の `tag_name` が `"template"` でない場合
    ///   (release + debug 共通)。html5ever
    ///   [`ElementFlags::template`](https://docs.rs/markup5ever/latest/markup5ever/interface/tree_builder/struct.ElementFlags.html#structfield.template)
    ///   が true になるのは HTML namespace の `<template>` element のみ、
    ///   したがってこの entry point は template element 限定。誤って通常
    ///   element を渡すのは caller bug (codex final review 2026-07-19 で
    ///   surface)。
    /// - `template_id` の `template_contents` slot が既に populate されている
    ///   場合 (debug のみ)。sink は template element ごとに 1 度だけこの
    ///   method を呼ぶ契約で、二重呼び出しは古い fragment root を silently
    ///   orphan するため debug で fail。release では上書きを許容
    ///   (M2+ mutation runtime での再 wire を想定した保守的挙動)。
    ///
    /// Returns: 新規 allocate された fragment root の arena index。
    pub fn allocate_template_fragment_root(&mut self, template_id: usize) -> usize {
        // Precondition: template_id は Element kind、かつ tag_name == "template"、
        // かつ template_contents slot が未 populate。
        // 借用の都合で immutable check を先に走らせて validation を確定させる
        // (Step 1 の append_element が &mut self を borrow するため)。
        {
            let data = match &self.nodes[template_id].data {
                NodeData::Element(e) => e.as_ref(),
                _ => panic!("allocate_template_fragment_root called on non-Element"),
            };
            assert_eq!(
                data.tag_name.as_str(),
                "template",
                "allocate_template_fragment_root called on non-<template> element (tag = {:?})",
                data.tag_name.as_str(),
            );
            debug_assert!(
                data.template_contents.is_none(),
                "allocate_template_fragment_root called twice on the same template \
                 (would orphan the previous fragment root at arena index {:?})",
                data.template_contents,
            );
        }
        // Step 1: fragment root を append_element(None) で detached allocate。
        // 内部で `flags_dirty=true` が set されるので、後段 `mark_in_document_flags`
        // が step 1 で fragment root の default IS_IN_DOCUMENT bit を clear する。
        let frag_root = self.append_element(
            None::<usize>,
            "#document-fragment",
            Style::default(),
            None::<&str>,
        );
        // Step 2: template element の template_contents slot に fragment root
        // index を wire。precondition check 済のため as_element_mut / template
        // tag_name の re-validation は不要。
        let e = self.nodes[template_id]
            .data
            .as_element_mut()
            .expect("allocate_template_fragment_root: element vanished between checks");
        e.template_contents = Some(frag_root);
        frag_root
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
            self.flags_dirty = true;
        }
    }

    /// `<template>` element の子孫について `IS_IN_DOCUMENT` bit を clear する
    /// (raikiri-spike-37c)。sink.finish() および mutation batch 後に呼ばれる。
    ///
    /// アルゴリズム (roborev job 292 findings 対応):
    /// 1. 全 arena node の bit を先に clear (detached / unreachable node を
    ///    default true のまま残さないため)
    /// 2. Document root から iterative DFS で bit set。template element 自身
    ///    は set、その descendants は skip (bit clear の状態が残る)
    ///
    /// 補足 (raikiri-spike-xno Part 2 併存): sink 経由の parse では template
    /// contents は fragment root subtree に流れ、Document root から reachable
    /// でなくなる → step 2 の DFS は自動的に届かない (in_template branch は
    /// 走らない)。だが本 step 2 の "template 判定 → descendants skip" logic は
    /// 残す: 手動で `append_element(Some(tmpl), ...)` を呼ぶ code path (raikiri-dom
    /// 内 test / raikiri-paint hello-world setup / 将来の M2+ mutation runtime
    /// で fragment root を経由しない contents 追加) は template 直下に子を積む
    /// ため、その inert 保証を defense-in-depth として維持する。
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
    /// - roborev job 294 M1 finding: `flags_dirty` が false のときは no-op
    ///   (idempotent + O(1))。mutation primitive が dirty mark するため、
    ///   observation-side は毎回本 method を呼んでも overhead が amortize される。
    ///   Consumer は「mutation batch → mark → observation」の contract を守る
    ///   ことでどこかの primitive で flag 更新を忘れた場合の regression を回避
    ///   できる。
    pub fn mark_in_document_flags(&mut self) {
        // roborev job 294 M1 finding: dirty check で cheap early return。
        // parse.finish() 直後 (dirty) → 明示的 recompute。以降 mutation 無しで
        // 複数回呼ばれても再計算しない。
        if !self.flags_dirty {
            return;
        }
        self.flags_dirty = false;
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

        assert!(
            doc.get_node(tmpl).unwrap().is_in_document(),
            "template stays in doc"
        );
        assert!(
            !doc.get_node(inner).unwrap().is_in_document(),
            "<p> cleared"
        );
        assert!(
            !doc.get_node(text).unwrap().is_in_document(),
            "text cleared"
        );
    }

    #[test]
    fn append_operations_set_flags_dirty() {
        // roborev job 294 M1 finding pin: mutation primitives が flags_dirty を
        // set することで、observation-side が mark_in_document_flags を呼ぶ contract
        // に依存できる。fresh Document は dirty=false からスタート。
        let mut doc = Document::new();
        assert!(!doc.flags_dirty, "fresh Document has clean flags");
        let e = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
        assert!(doc.flags_dirty, "append_element sets dirty");
        doc.flags_dirty = false;
        doc.append_text(e, "hi");
        assert!(doc.flags_dirty, "append_text sets dirty");
    }

    #[test]
    fn attach_and_detach_set_flags_dirty() {
        // attach_child / detach_from_parent / reparent_children / insert_child_before /
        // retain_children はいずれも tree topology を変えるため flags_dirty を
        // set する契約。
        let mut doc = Document::new();
        let a = doc.append_element(Some(0), "a", Style::default(), None::<&str>);
        let b = doc.append_element(Some(0), "b", Style::default(), None::<&str>);
        let d = doc.append_element(None::<usize>, "d", Style::default(), None::<&str>);
        doc.mark_in_document_flags(); // clean
        assert!(!doc.flags_dirty);

        doc.attach_child(a, d);
        assert!(doc.flags_dirty, "attach_child sets dirty");
        doc.mark_in_document_flags();

        doc.detach_from_parent(d);
        assert!(doc.flags_dirty, "detach_from_parent sets dirty");
        doc.mark_in_document_flags();

        doc.attach_child(a, d);
        doc.mark_in_document_flags();
        doc.reparent_children(a, b);
        assert!(doc.flags_dirty, "reparent_children sets dirty");
        doc.mark_in_document_flags();

        doc.retain_children(|c| c != d);
        assert!(
            doc.flags_dirty,
            "retain_children sets dirty when a child is removed"
        );
    }

    #[test]
    fn mark_in_document_flags_is_noop_when_clean() {
        // roborev job 294 M1 finding: mark_in_document_flags は !flags_dirty のとき
        // 何もしない。invariant: 一度 mark した後 mutation が無ければ再 mark は
        // 高速で idempotent。
        let mut doc = Document::new();
        let e = doc.append_element(Some(0), "e", Style::default(), None::<&str>);
        doc.mark_in_document_flags();
        assert!(!doc.flags_dirty);
        assert!(doc.get_node(e).unwrap().is_in_document());
        // 再 mark は no-op、状態不変。
        doc.mark_in_document_flags();
        assert!(doc.get_node(e).unwrap().is_in_document());
        assert!(!doc.flags_dirty);
    }

    #[test]
    fn set_element_namespace_dirties_flags_for_template_only() {
        // roborev job 295 M2 finding pin: `set_element_namespace` は
        // `<template>` element の namespace を変更した場合のみ flags_dirty を
        // set する。template 以外は set しない (pure metadata、layout 無影響)。
        let mut doc = Document::new();
        let tmpl = doc.append_element(Some(0), "template", Style::default(), None::<&str>);
        let div = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
        doc.mark_in_document_flags();
        assert!(!doc.flags_dirty);

        // template の namespace を変更 → dirty set される
        doc.set_element_namespace(tmpl, Some(SmolStr::new("http://www.w3.org/2000/svg")));
        assert!(
            doc.flags_dirty,
            "template namespace change must dirty flags"
        );
        doc.mark_in_document_flags();
        assert!(!doc.flags_dirty);

        // div の namespace を変更 → dirty set されない (template 判定に無関係)
        doc.set_element_namespace(div, Some(SmolStr::new("http://www.w3.org/2000/svg")));
        assert!(
            !doc.flags_dirty,
            "non-template namespace change must NOT dirty flags"
        );

        // 同じ namespace を再度 set → 変化無しなら dirty set しない
        doc.set_element_namespace(tmpl, Some(SmolStr::new("http://www.w3.org/2000/svg")));
        assert!(!doc.flags_dirty, "no-op namespace set must NOT dirty flags");
    }

    #[test]
    fn post_mark_attach_under_template_becomes_out_of_document_after_remark() {
        // roborev job 294 M1 finding: mutation → 再 mark で正しい bit 状態が復元
        // されることを end-to-end で pin。post-parse mutation の contract。
        let mut doc = Document::new();
        let root = doc.root_index();
        let tmpl = doc.append_element(Some(root), "template", Style::default(), None::<&str>);
        doc.mark_in_document_flags();
        assert!(doc.get_node(tmpl).unwrap().is_in_document());

        // 新規 append_element は default IS_IN_DOCUMENT=true で作られる。
        // template 配下に attach するので flags_dirty=true になり、mark で
        // false に落ちるべき。
        let new_child = doc.append_element(Some(tmpl), "p", Style::default(), None::<&str>);
        assert!(doc.flags_dirty, "mutation → dirty");
        // mark 前は default true (bit reset は mark でしか起きない)
        assert!(doc.get_node(new_child).unwrap().is_in_document());
        doc.mark_in_document_flags();
        assert!(
            !doc.get_node(new_child).unwrap().is_in_document(),
            "child attached under template must become out-of-document after remark"
        );
    }
}

#[cfg(test)]
mod find_body_flat_tree_tests {
    // roborev job 295 M3 finding pin: find_body (both layout and paint impls)
    // must not select a <body> that lives inside an inert subtree
    // (<template>...<body>ghost</body>...</template>).
    use super::*;
    use crate::layout::find_body as layout_find_body;

    #[test]
    fn layout_find_body_skips_body_inside_template() {
        let mut doc = Document::new();
        let root = doc.root_index();
        let html = doc.append_element(Some(root), "html", Style::default(), None::<&str>);
        // Ghost body under template (should NOT be picked)
        let tmpl = doc.append_element(Some(html), "template", Style::default(), None::<&str>);
        let _ghost = doc.append_element(Some(tmpl), "body", Style::default(), None::<&str>);
        // Real body under html (should be picked)
        let real = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        doc.mark_in_document_flags();

        assert_eq!(
            layout_find_body(&doc),
            Some(real),
            "find_body must skip inert body inside <template> and select the real body"
        );
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
