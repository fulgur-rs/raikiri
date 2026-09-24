//! Document arena — Vec-backed node arena that implements taffy layout traits
//! (in `taffy_impl.rs`) and raikiri-traits::Dom (in `dom_impl.rs`).
//!
//! # Flat tree membership contract
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
use std::sync::Arc;
use taffy::Style;

use raikiri_traits::{QuirksMode, StylesheetKind};

use crate::fragment::{FragmentTree, FragmentationContext};
use crate::layout::LayoutWarn;
use crate::node::{Attr, Node, NodeData};
use raikiri_style::property::CalcLengthPercentage;

const XHTML_NAMESPACE_URI: &str = "http://www.w3.org/1999/xhtml";

fn is_xml_name_start(ch: char) -> bool {
    matches!(
        ch,
        ':' | 'A'..='Z'
            | '_'
            | 'a'..='z'
            | '\u{C0}'..='\u{D6}'
            | '\u{D8}'..='\u{F6}'
            | '\u{F8}'..='\u{2FF}'
            | '\u{370}'..='\u{37D}'
            | '\u{37F}'..='\u{1FFF}'
            | '\u{200C}'..='\u{200D}'
            | '\u{2070}'..='\u{218F}'
            | '\u{2C00}'..='\u{2FEF}'
            | '\u{3001}'..='\u{D7FF}'
            | '\u{F900}'..='\u{FDCF}'
            | '\u{FDF0}'..='\u{FFFD}'
            | '\u{10000}'..='\u{EFFFF}'
    )
}

fn is_xml_name_char(ch: char) -> bool {
    is_xml_name_start(ch)
        || matches!(
            ch,
            '0'..='9' | '-' | '.' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}'
        )
}

fn is_valid_xml_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(is_xml_name_start) && chars.all(is_xml_name_char)
}

fn is_html_raw_text_element(namespace: Option<&str>, tag_name: &str) -> bool {
    namespace.is_none_or(|namespace| namespace == XHTML_NAMESPACE_URI)
        && matches!(
            tag_name.to_ascii_lowercase().as_str(),
            "script"
                | "style"
                | "xmp"
                | "iframe"
                | "noembed"
                | "noframes"
                | "plaintext"
                | "noscript"
        )
}

/// DOM Document (root + Vec-backed node arena)。
///
/// `nodes` は arena indices を key とする flat storage。index 0 は Document
/// kind の virtual root。HTML の `<html>` element は raikiri-html の基本
/// parse 経路が index 1 以降に append する想定 (root = 0 の子として)。
#[derive(Debug, Clone)]
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
    /// Cascade generation used by the most recent successful layout. Resolved
    /// order projections and Grid row placements are valid only for this run.
    pub(crate) layout_cascade_generation: Option<u64>,
    /// IS_IN_DOCUMENT bit dirty flag。任意の tree-mutation primitive
    /// (append_* / attach_child / insert_child_before / detach_from_parent /
    /// reparent_children / retain_children) で set される。observation-side
    /// API (cascade / paint / extract) は生の bit を信じる前に
    /// [`Document::mark_in_document_flags`] を呼ぶことで dirty check + lazy
    /// recompute を強制する contract。
    ///
    /// 現状の parse-only 経路では `sink.finish()` が明示的に呼ぶため無視できる
    /// が、手動で `append_*` を呼んで Document を組み立てる code path
    /// (raikiri-dom 内 test / raikiri-paint hello-world setup / 将来の
    /// mutation runtime) では本 dirty flag が correctness の contract を担う。
    pub(crate) flags_dirty: bool,
    /// Document に associate されている stylesheet の list。
    /// lazy: parse は cascade phase で行う。
    /// 呼び出し順で同 kind 内の cascade source_order が決まる。
    stylesheets: Vec<(Cow<'static, str>, StylesheetKind)>,
    /// Buffered [`LayoutWarn`] diagnostic events for the current (or most
    /// recent) `layout_single_page` pass (generalizing
    /// the `fonts.rs` `FontWarn` observer pattern to this crate's other
    /// "silent clamp" site).
    ///
    /// Owned (`Vec`, no borrowed observer) rather than a closure field —
    /// deliberately, not as a simplification of convenience. `<Document as
    /// taffy::LayoutPartialTree>::set_unrounded_layout` is the sole choke
    /// point that writes non-finite-clamped geometry into the arena
    /// ([`crate::layout::sanitize_taffy_layout`]'s doc), but its signature is
    /// fixed by the `taffy` trait — it cannot receive an extra observer
    /// parameter. Storing a borrowed `&mut dyn FnMut` here instead would
    /// require adding a lifetime parameter to `Document` itself, which is a
    /// public-shape break every consumer of this type would have to absorb
    /// (dom→paint wall territory) for a capability nothing external can
    /// plug into yet. An owned buffer sidesteps that: `set_unrounded_layout`
    /// pushes through `self` with no signature change, and
    /// `layout_single_page` drains + replays the buffer through the same
    /// [`crate::diag::emit_warn_via`] mechanism the rest of this module's
    /// diagnostics use, once per pass, after the taffy compute step returns.
    ///
    /// Cleared at the start of each `layout_single_page` call (re-entrance
    /// safety, mirrors the `Node.text_layout` clear in the same function) and
    /// drained near its end.
    ///
    /// # Scope boundary: only `layout_single_page` clears/drains this
    ///
    /// A `Document` driven through `taffy::compute_root_layout` directly
    /// (bypassing `layout_single_page` — e.g. this crate's own `lib.rs` unit
    /// tests) still has `set_unrounded_layout` pushing into this buffer, but
    /// nothing clears or drains it. [`LAYOUT_WARN_CAP`]-plus-one bounds the
    /// memory either way, so this is not a leak, but on such a `Document` the
    /// first pathological layout pass fills the buffer and every event after
    /// that collapses into the trailing `Truncated` counter, with nothing
    /// ever reading it back out. Not a problem for `layout_single_page`
    /// callers (the only production path); worth knowing if a future
    /// consumer drives taffy directly and expects these diagnostics.
    ///
    /// [`LAYOUT_WARN_CAP`]: crate::layout::LAYOUT_WARN_CAP
    pub(crate) layout_warnings: Vec<LayoutWarn>,
    /// Stable storage for Taffy calc resolver payloads used by the current
    /// layout pass. The heap allocations keep pointees stable while styles hold raw handles.
    pub(crate) calc_values: Vec<Arc<CalcLengthPercentage>>,
    /// Fragments emitted by the active multicol strategy for this layout pass.
    pub(crate) fragment_tree: FragmentTree,
    /// Active nested fragmentainer stack while Taffy recursively lays out nodes.
    pub(crate) fragmentation_stack: Vec<FragmentationContext>,
    /// HTML5 quirks mode for this whole document. Default
    /// [`QuirksMode::NoQuirks`] (matching the type's own `#[default]`) for
    /// `Document`s built by hand (raikiri-dom unit tests, raikiri-paint
    /// hello-world setup). raikiri-html's parse sink calls
    /// [`Document::set_quirks_mode`] with the html5ever-detected value
    /// before handing the `Document` off, so parsed documents carry their
    /// real value. Read back via [`Document::quirks_mode`] and by
    /// `impl raikiri_style::StyleDom for Document`'s `quirks_mode()`
    /// override (`dom_impl.rs`), which converts it to
    /// `raikiri_style::StyleQuirksMode` for cascade's id/class
    /// case-folding (CSS Selectors L4).
    quirks_mode: QuirksMode,
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
            layout_cascade_generation: None,
            // 初期 root node は Node::new_document() が IS_IN_DOCUMENT=true を
            // 立てているため、"attached under root" として consistent。まだ
            // template も detached node も無いので dirty ではない。
            flags_dirty: false,
            stylesheets: Vec::new(),
            layout_warnings: Vec::new(),
            calc_values: Vec::new(),
            fragment_tree: FragmentTree::default(),
            fragmentation_stack: Vec::new(),
            quirks_mode: QuirksMode::default(),
        }
    }

    /// Element node を arena に追加する。`parent` が `Some(idx)` の場合
    /// その node の children に append される。`None` の場合 detached (どこにも
    /// 属さない fragment、後で attach する用途)。
    ///
    /// `inline_style` は HTML `style="..."` attribute の生 string を渡す
    /// (`None` = 属性なし)。raikiri-style::cascade が消費する。
    ///
    /// # Contract
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

    /// Append character data, merging it into the parent's adjacent final text
    /// node when possible. HTML token streams may split one logical character
    /// run into several callbacks; coalescing those callbacks preserves the
    /// inline formatting run without changing the ordinary [`Document::append_text`]
    /// mutation primitive.
    pub fn append_text_coalesced(&mut self, parent: usize, text: impl Into<SmolStr>) -> usize {
        let text = text.into();
        if let Some(&last) = self.nodes[parent].children.last()
            && let NodeData::Text(data) = &mut self.nodes[last].data
        {
            let mut merged = String::with_capacity(data.text_content.len() + text.len());
            merged.push_str(data.text_content.as_str());
            merged.push_str(text.as_str());
            data.text_content = SmolStr::new(merged);
            self.invalidate_layout_cache();
            self.flags_dirty = true;
            return last;
        }
        self.append_text(parent, text)
    }

    /// Comment node を arena に追加する。
    ///
    /// `parent` が `Some(idx)` の場合その node の children に append される。
    /// `None` の場合 detached (html5ever `TreeSink::create_comment` の primitive
    /// と対応 — html5ever は comment を detached に作ってから後で
    /// `append(parent, AppendNode(c))` する)。
    ///
    /// # Flat tree semantics
    ///
    /// Comment は `NodeKind::Element` ではなく `NodeKind::Comment` なので
    /// cascade / paint / stylesheet extraction は Element gate で自動 skip する。
    /// 追加で [`Document::mark_in_document_flags`] が Comment / PI variant を
    /// 観測すると `IS_IN_DOCUMENT` bit を clear する contract により、Taffy layout
    /// tree (is_in_document filter) からも自動的に消える。旧来の
    /// `strip_non_element_stubs` (arena children Vec からの physical 除去) は
    /// この 2 段 gate に置き換わったため raikiri-html sink から廃止した。
    ///
    /// Returns: 追加された node の arena index。
    pub fn append_comment(&mut self, parent: Option<usize>, text: impl Into<SmolStr>) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node::new_comment(text.into()));
        if let Some(p) = parent {
            self.nodes[p].children.push(id);
        }
        self.invalidate_layout_cache();
        self.flags_dirty = true;
        id
    }

    /// Processing instruction node を arena に追加する。
    ///
    /// `parent` は Comment と同じ semantics (Some で attach、None で detached)。
    /// HTML では実質発生しないが XML / XHTML では有効な NodeType (WHATWG DOM §4)。
    ///
    /// Flat tree semantics: Comment と同一 (Element でないので cascade / paint /
    /// extract の Element gate で skip、`IS_IN_DOCUMENT` clear で taffy からも
    /// 消える)。
    ///
    /// Returns: 追加された node の arena index。
    pub fn append_processing_instruction(
        &mut self,
        parent: Option<usize>,
        target: impl Into<SmolStr>,
        data: impl Into<SmolStr>,
    ) -> usize {
        let id = self.nodes.len();
        self.nodes
            .push(Node::new_processing_instruction(target.into(), data.into()));
        if let Some(p) = parent {
            self.nodes[p].children.push(id);
        }
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
    ///
    /// # Fragment-aware semantics (WHATWG DOM §4.2.3 Mutation algorithms)
    ///
    /// `child` が [`NodeData::DocumentFragment`] variant の場合、fragment node
    /// 自身は `parent.children` に append せず、fragment の全 children を parent
    /// の末尾に移動する (fragment の children は afterwards 空になる、
    /// insert algorithm steps 1 + 4.1 + 7.2 —
    /// step 1 で fragment の場合 nodes = fragment.children、step 4.1 で fragment の
    /// children を drain、step 7.2 で parent.children の末尾に append
    /// (spec は append を "pre-insert node into parent before null" と定義するので
    /// 本 method は referenceChild = null 経路 = step 7.2 分岐、non-null
    /// referenceChild の positional splice は step 7.3 で
    /// [`Document::insert_child_before`] 側が該当))。これは
    /// spec-conformant な DocumentFragment insertion semantics で、fragment そのもの
    /// は常に unrendered な virtual container として振る舞う。
    ///
    /// Element / Text / Comment / PI / Document は fragment 以外なので直接 append
    /// (旧挙動保持)。html5ever が実行時に fragment を parent として渡すことは
    /// ない (fragment は `get_template_contents` の返り値になる parent 側のみで
    /// child 側では現れない) ため、この分岐は raikiri-dom を直接 driving する
    /// consumer (test / 将来の DOM Mutation API) のためのもの。
    ///
    /// # Panics
    ///
    /// - `parent` / `child` が arena 範囲外 (`nodes[..]` indexing による)。
    /// - `parent == child` の場合 (spec HierarchyRequestError 相当) は現状
    ///   detect しない (現在の実装範囲では発生しない、将来 spec-conformant
    ///   mutation API 化する時に raise 判定する予定)。
    pub fn attach_child(&mut self, parent: usize, child: usize) {
        // Fragment-aware branch: WHATWG DOM insert algorithm steps 1 + 4.1 + 7.2
        // (§4.2.3 Mutation algorithms) の効果と一致 — fragment 自身は
        // parent.children に含めず、fragment の children を parent の末尾へ
        // move する。insert_child_before (positional splice) の tail append 対応。
        if matches!(self.nodes[child].data, NodeData::DocumentFragment) {
            // reparent_children の drain + extend pattern と一致。fragment 自身
            // は `parent.children` に含まれない (contract test (c) 参照)。
            let moved: Vec<usize> = self.nodes[child].children.drain(..).collect();
            self.nodes[parent].children.extend(moved);
        } else {
            self.nodes[parent].children.push(child);
        }
        self.invalidate_layout_cache();
        self.flags_dirty = true;
    }

    /// `parent` の children 配列内、`before` の直前 index に `child` を挿入する。
    ///
    /// html5ever `TreeSink::append_before_sibling(sibling, AppendNode(child))`
    /// および foster parenting の primitive。`before` が `parent` の子でない場合
    /// は末尾に append する (defensive: TreeSink 規約上発生しない想定)。
    ///
    /// # Fragment-aware semantics (WHATWG DOM §4.2.3 Mutation algorithms)
    ///
    /// `child` が [`NodeData::DocumentFragment`] variant の場合、fragment node
    /// 自身は `parent.children` に挿入せず、fragment の全 children を parent の
    /// `before` position から source order で splice する (fragment の children は
    /// afterwards 空になる、insert algorithm steps 1 + 4.1 + 7.3 —
    /// step 1 で fragment の場合 nodes = fragment.children、step 4.1 で fragment の
    /// children を drain、step 7.3 で referenceChild の index に splice
    /// (step 7.2 は null referenceChild = tail append case、本 method は
    /// non-null referenceChild 経路なので step 7.3 分岐))。これは
    /// spec-conformant な DocumentFragment insertion semantics で、fragment そのもの
    /// は常に unrendered な virtual container として振る舞う。
    ///
    /// Element / Text / Comment / PI / Document は fragment 以外なので直接 insert
    /// (旧挙動保持)。html5ever が実行時に fragment を `append_before_sibling` の
    /// new_node として渡すことはない (fragment は `get_template_contents` の返り値
    /// になる parent 側のみで child 側では現れない) ため、この分岐は raikiri-dom
    /// を直接 driving する consumer (test / 将来の DOM Mutation API) のためのもの
    /// (以前 latent な asymmetry として観測されていたが、後に
    /// [`Document::attach_child`] と整合するよう修正済み)。
    pub fn insert_child_before(&mut self, parent: usize, before: usize, child: usize) {
        // Fragment-aware branch: WHATWG DOM insert algorithm steps 1 + 4.1 + 7.3
        // (§4.2.3 Mutation algorithms) の効果と一致 — fragment 自身は
        // parent.children に含めず、fragment の children を `before` position から
        // source order で splice する。attach_child (tail append) の positional 対応。
        if matches!(self.nodes[child].data, NodeData::DocumentFragment) {
            // drain fragment children (move semantics) — collect() で borrow を切り、
            // 後段の parent.children への &mut と衝突しない。
            let moved: Vec<usize> = self.nodes[child].children.drain(..).collect();
            let kids = &mut self.nodes[parent].children;
            let pos = kids.iter().position(|&c| c == before).unwrap_or_else(|| {
                // html5ever TreeSink contract 上ここには来ない。dev/test では
                // contract 違反として panic、release では末尾追加 fallback。
                debug_assert!(
                    false,
                    "insert_child_before: `before` ({before}) not a child of parent ({parent})"
                );
                kids.len()
            });
            // Vec::splice(pos..pos, moved) は pos 位置に moved を単一 pass で
            // 挿入する (何も remove しない)。O(n+k) allocation で完了。
            kids.splice(pos..pos, moved);
        } else {
            let kids = &mut self.nodes[parent].children;
            if let Some(pos) = kids.iter().position(|&c| c == before) {
                kids.insert(pos, child);
            } else {
                // html5ever TreeSink contract 上ここには来ない。dev/test では
                // contract 違反として panic、release では plan 指定の tail-append
                // fallback。
                debug_assert!(
                    false,
                    "insert_child_before: `before` ({before}) not a child of parent ({parent})"
                );
                kids.push(child);
            }
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

    /// Element node に non-HTML namespace URI を紐付ける。
    /// `ns` が `None` = HTML default namespace / element
    /// でない場合の効果無し。HTML default は `None` を optimized path とする
    /// (memory saving + `Element::namespace_uri()` の O(1) 判定)。
    ///
    /// raikiri-html sink が `finish()` 時に qual_names metadata table から呼び出す。
    /// tree mutation ではないので `invalidate_layout_cache` は call しない。
    ///
    /// Panics (debug + release 共通): `id` が Element kind
    /// でない場合。Text / Document node に attribute-family setter を呼ぶのは
    /// caller bug なので early fail させる (旧 `debug_assert_eq!` から
    /// `NodeData::as_element_mut().expect(...)` に移行、release でも panic する
    /// ようになったのは意図的な strictness 向上)。
    pub fn set_element_namespace(&mut self, id: usize, ns: Option<SmolStr>) {
        // namespace の変更は
        // `<template>` 判定 (`namespace.is_none()` は HTML default optimized path) を
        // 変え得るため、tag_name が "template" の場合は flags_dirty を set する。
        // これがないと HTML template → SVG template への変更 (あるいは逆) の後
        // `mark_in_document_flags()` が early return path で no-op となり、
        // 子孫の in_document bit が stale のまま残る。
        //
        // template 以外の element では namespace 変更は本 bit に無関係なので
        // flag は set しない (invalidate_layout_cache も呼ばない: pure metadata
        // 変更で layout 結果を変えない、既存の attribute-setter 契約と一貫)。
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

    /// Element node に attribute list を紐付ける。
    /// `attrs` は null-namespace attribute の `(local, value)` 列。html5ever の
    /// source order を保持する必要があるので Vec で受ける。`style` attribute は
    /// [`Document::set_element_inline_style`] で別途 wire するため呼び出し側で
    /// 除外しておくこと。
    ///
    /// raikiri-html sink が `finish()` 時に attributes metadata table から呼び出す。
    /// tree mutation ではないので `invalidate_layout_cache` は call しない。
    ///
    /// Panics (debug + release 共通): `id` が Element kind
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

    /// Set one null-namespace attribute on an element.
    ///
    /// The first existing entry keeps its source-order position; duplicate
    /// entries with the same local name are removed. A missing attribute is
    /// appended. The `style` attribute is stored in the separate
    /// inline-style slot used by [`Document::set_element_inline_style`], never
    /// in `ElementData::attributes`.
    ///
    /// This updates attribute metadata only; like [`Document::set_element_attributes`],
    /// it does not invalidate layout caches or mark tree membership dirty.
    ///
    /// Returns an error when `local` is not an XML name. HTML-namespace element
    /// names are ASCII-lowercased; foreign-content names preserve their case.
    ///
    /// Panics (debug + release): `id` is not an Element.
    pub fn set_element_attribute(
        &mut self,
        id: usize,
        local: impl Into<SmolStr>,
        value: impl Into<SmolStr>,
    ) -> Result<(), String> {
        let local = local.into();
        if !is_valid_xml_name(local.as_str()) {
            return Err(format!("invalid attribute name: {local}"));
        }
        let NodeData::Element(element) = &self.nodes[id].data else {
            panic!("set_element_attribute called on non-Element");
        };
        let html_element = element
            .namespace
            .as_deref()
            .is_none_or(|namespace| namespace == XHTML_NAMESPACE_URI);
        let local: SmolStr = if html_element {
            local.to_ascii_lowercase().into()
        } else {
            local
        };
        let value = value.into();
        if local.as_str() == "style" {
            // Keep the storage invariant even if a caller previously populated
            // the full-list setter with a style entry by mistake.
            let element = self.nodes[id]
                .data
                .as_element_mut()
                .expect("set_element_attribute called on non-Element");
            element.attributes.retain(|attr| attr.local != local);
            self.set_element_inline_style(id, Some(value));
            return Ok(());
        }

        let element = self.nodes[id]
            .data
            .as_element_mut()
            .expect("set_element_attribute called on non-Element");
        let mut found = false;
        element.attributes.retain_mut(|attr| {
            if attr.local == local {
                if found {
                    false
                } else {
                    found = true;
                    attr.value = value.clone();
                    true
                }
            } else {
                true
            }
        });
        if !found {
            element.attributes.push(Attr { local, value });
        }
        Ok(())
    }

    /// Remove one null-namespace attribute from an element and return its
    /// stored value, if present. All matching entries are removed. The
    /// separate inline-style slot is used for `style` and is cleared by this
    /// method as well.
    ///
    /// This updates attribute metadata only; it does not invalidate layout
    /// caches or mark tree membership dirty.
    ///
    /// Returns an error when `local` is not an XML name. HTML-namespace element
    /// names are ASCII-lowercased; foreign-content names preserve their case.
    ///
    /// Panics (debug + release): `id` is not an Element.
    pub fn remove_element_attribute(
        &mut self,
        id: usize,
        local: &str,
    ) -> Result<Option<SmolStr>, String> {
        if !is_valid_xml_name(local) {
            return Err(format!("invalid attribute name: {local}"));
        }
        let NodeData::Element(element) = &self.nodes[id].data else {
            panic!("remove_element_attribute called on non-Element");
        };
        let html_element = element
            .namespace
            .as_deref()
            .is_none_or(|namespace| namespace == XHTML_NAMESPACE_URI);
        let local = if html_element {
            local.to_ascii_lowercase()
        } else {
            local.to_owned()
        };
        let element = self.nodes[id]
            .data
            .as_element_mut()
            .expect("remove_element_attribute called on non-Element");
        if local == "style" {
            // Remove any legacy/misrouted list entries as well as the actual
            // inline-style value, so `attr("style")` has one source of truth.
            let legacy_value = element
                .attributes
                .iter()
                .find(|attr| attr.local.as_str() == local)
                .map(|attr| attr.value.clone());
            element
                .attributes
                .retain(|attr| attr.local.as_str() != local);
            return Ok(element.inline_style.take().or(legacy_value));
        }

        let first_value = element
            .attributes
            .iter()
            .find(|attr| attr.local.as_str() == local)
            .map(|attr| attr.value.clone());
        element
            .attributes
            .retain(|attr| attr.local.as_str() != local);
        Ok(first_value)
    }

    /// Replace `target_parent`'s children with deep copies of
    /// `source_parent`'s children from another document.
    ///
    /// Existing target child nodes stay allocated in the arena, but are
    /// detached from their parent. New elements preserve their tag name,
    /// namespace, null-namespace attributes, inline style, and source order;
    /// text, comments, and processing instructions are copied as nodes.
    /// Template contents are copied into a fresh detached fragment root.
    /// Attribute `style` remains in the separate inline-style slot.
    ///
    /// Tree changes go through the existing `detach_from_parent`, `append_*`,
    /// and template-fragment mutation methods, so layout caches and flat-tree
    /// membership are marked dirty in the usual way. The source document is
    /// not modified.
    pub fn replace_children_from(
        &mut self,
        target_parent: usize,
        source_document: &Document,
        source_parent: usize,
    ) {
        let target_parent = self.nodes[target_parent]
            .template_contents()
            .unwrap_or(target_parent);
        let old_children = self.nodes[target_parent].children.clone();
        for child in old_children {
            self.detach_from_parent(child);
        }

        // A LIFO worklist avoids recursion on deeply nested parsed documents.
        // Push siblings in reverse so each subtree is copied in source order.
        let mut pending: Vec<(usize, usize)> = source_document.nodes[source_parent]
            .children
            .iter()
            .rev()
            .map(|&child| (child, target_parent))
            .collect();

        while let Some((source_id, target_parent)) = pending.pop() {
            let source_node = &source_document.nodes[source_id];
            let source_children = source_node.children.clone();

            match &source_node.data {
                NodeData::Element(element) => {
                    let new_id = self.append_element(
                        Some(target_parent),
                        element.tag_name.clone(),
                        source_node.style.clone(),
                        element.inline_style.clone(),
                    );
                    self.set_element_namespace(new_id, element.namespace.clone());
                    self.set_element_attributes(
                        new_id,
                        element
                            .attributes
                            .iter()
                            .filter(|attr| attr.local.as_str() != "style")
                            .map(|attr| (attr.local.clone(), attr.value.clone()))
                            .collect(),
                    );

                    if let Some(source_fragment) = element.template_contents {
                        let target_fragment = self.allocate_template_fragment_root(new_id);
                        pending.extend(
                            source_document.nodes[source_fragment]
                                .children
                                .iter()
                                .rev()
                                .map(|&child| (child, target_fragment)),
                        );
                    }
                    pending.extend(source_children.iter().rev().map(|&child| (child, new_id)));
                }
                NodeData::Text(text) => {
                    let new_id = self.append_text(target_parent, text.text_content.clone());
                    pending.extend(source_children.iter().rev().map(|&child| (child, new_id)));
                }
                NodeData::Comment(text) => {
                    let new_id = self.append_comment(Some(target_parent), text.clone());
                    pending.extend(source_children.iter().rev().map(|&child| (child, new_id)));
                }
                NodeData::ProcessingInstruction { target, data } => {
                    let new_id = self.append_processing_instruction(
                        Some(target_parent),
                        target.clone(),
                        data.clone(),
                    );
                    pending.extend(source_children.iter().rev().map(|&child| (child, new_id)));
                }
                // A DocumentFragment is a container, not a child node. Match
                // DOM insertion semantics by splicing its children in place.
                NodeData::DocumentFragment => {
                    pending.extend(
                        source_children
                            .iter()
                            .rev()
                            .map(|&child| (child, target_parent)),
                    );
                }
                NodeData::Document => {
                    panic!("replace_children_from cannot copy a Document node as a child");
                }
            }
        }
    }

    /// Serialize a node's children as an HTML fragment using the current arena
    /// state. This is suitable for reading live `innerHTML`; it does not use or
    /// retain an original source string.
    ///
    /// Element attributes and the separate inline-style slot are escaped and
    /// emitted from their current values. Text is escaped, comments and
    /// processing instructions are preserved, and HTML void elements are
    /// emitted without end tags. Template elements serialize their contents
    /// fragment rather than ordinary children.
    ///
    /// Returns an error for an out-of-range parent, a non-container parent, or
    /// malformed child/template-fragment indices in the arena.
    pub fn serialize_inner_html(&self, parent: usize) -> Result<String, String> {
        fn push_escaped(output: &mut String, value: &str, attribute: bool) {
            for ch in value.chars() {
                match ch {
                    '&' => output.push_str("&amp;"),
                    '<' => output.push_str("&lt;"),
                    '>' => output.push_str("&gt;"),
                    '"' if attribute => output.push_str("&quot;"),
                    '\'' if attribute => output.push_str("&#39;"),
                    _ => output.push(ch),
                }
            }
        }

        enum Task {
            Node(usize, bool),
            EndTag(SmolStr),
        }

        let parent_node = self
            .nodes
            .get(parent)
            .ok_or_else(|| format!("innerHTML parent index {parent} is out of range"))?;
        if !matches!(
            &parent_node.data,
            NodeData::Document | NodeData::DocumentFragment | NodeData::Element(_)
        ) {
            return Err(format!(
                "innerHTML parent index {parent} is not a container node"
            ));
        }

        let initial_children = if let NodeData::Element(element) = &parent_node.data {
            if let Some(fragment_id) = element.template_contents {
                let fragment = self.nodes.get(fragment_id).ok_or_else(|| {
                    format!("template contents fragment index {fragment_id} is out of range")
                })?;
                if !matches!(&fragment.data, NodeData::DocumentFragment) {
                    return Err(format!(
                        "template contents index {fragment_id} is not a fragment"
                    ));
                }
                fragment.children.clone()
            } else {
                parent_node.children.clone()
            }
        } else {
            parent_node.children.clone()
        };
        let parent_raw_text = match &parent_node.data {
            NodeData::Element(element) => {
                is_html_raw_text_element(element.namespace.as_deref(), element.tag_name.as_str())
            }
            _ => false,
        };
        let mut output = String::new();
        let mut pending: Vec<Task> = initial_children
            .into_iter()
            .rev()
            .map(|child| Task::Node(child, parent_raw_text))
            .collect();

        while let Some(task) = pending.pop() {
            match task {
                Task::EndTag(tag) => {
                    output.push_str("</");
                    output.push_str(tag.as_str());
                    output.push('>');
                }
                Task::Node(id, raw_text_parent) => {
                    let node = self
                        .nodes
                        .get(id)
                        .ok_or_else(|| format!("innerHTML child index {id} is out of range"))?;
                    match &node.data {
                        NodeData::Element(element) => {
                            let tag = element.tag_name.as_str();
                            output.push('<');
                            output.push_str(tag);
                            for attr in &element.attributes {
                                // `style` is represented by inline_style and
                                // must have only one serialized source.
                                if attr.local.as_str() == "style" {
                                    continue;
                                }
                                if !is_valid_xml_name(attr.local.as_str()) {
                                    return Err(format!(
                                        "invalid attribute name {:?} on innerHTML node {id}",
                                        attr.local
                                    ));
                                }
                                output.push(' ');
                                output.push_str(attr.local.as_str());
                                output.push_str("=\"");
                                push_escaped(&mut output, attr.value.as_str(), true);
                                output.push('"');
                            }
                            if let Some(style) = &element.inline_style {
                                output.push_str(" style=\"");
                                push_escaped(&mut output, style.as_str(), true);
                                output.push('"');
                            }
                            output.push('>');

                            let is_html_void = element.namespace.is_none()
                                && matches!(
                                    tag.to_ascii_lowercase().as_str(),
                                    "area"
                                        | "base"
                                        | "br"
                                        | "col"
                                        | "embed"
                                        | "hr"
                                        | "img"
                                        | "input"
                                        | "link"
                                        | "meta"
                                        | "param"
                                        | "source"
                                        | "track"
                                        | "wbr"
                                );
                            if is_html_void {
                                continue;
                            }

                            let children = if let Some(fragment_id) = element.template_contents {
                                let fragment = self.nodes.get(fragment_id).ok_or_else(|| {
                                    format!(
                                        "template contents fragment index {fragment_id} is out of range"
                                    )
                                })?;
                                if !matches!(&fragment.data, NodeData::DocumentFragment) {
                                    return Err(format!(
                                        "template contents index {fragment_id} is not a fragment"
                                    ));
                                }
                                fragment.children.clone()
                            } else {
                                node.children.clone()
                            };
                            pending.push(Task::EndTag(element.tag_name.clone()));
                            let raw_text = is_html_raw_text_element(
                                element.namespace.as_deref(),
                                element.tag_name.as_str(),
                            );
                            pending.extend(
                                children
                                    .into_iter()
                                    .rev()
                                    .map(|child| Task::Node(child, raw_text)),
                            );
                        }
                        NodeData::Text(text) => {
                            if raw_text_parent {
                                output.push_str(text.text_content.as_str());
                            } else {
                                push_escaped(&mut output, text.text_content.as_str(), false);
                            }
                        }
                        NodeData::Comment(text) => {
                            output.push_str("<!--");
                            output.push_str(text.as_str());
                            output.push_str("-->");
                        }
                        NodeData::ProcessingInstruction { target, data } => {
                            output.push_str("<?");
                            output.push_str(target.as_str());
                            if !data.is_empty() {
                                output.push(' ');
                                output.push_str(data.as_str());
                            }
                            output.push_str("?>");
                        }
                        NodeData::DocumentFragment => {
                            pending.extend(
                                node.children
                                    .iter()
                                    .rev()
                                    .map(|&child| Task::Node(child, raw_text_parent)),
                            );
                        }
                        NodeData::Document => {
                            return Err(format!(
                                "innerHTML cannot serialize Document node {id} as a child"
                            ));
                        }
                    }
                }
            }
        }

        Ok(output)
    }

    /// `<template>` element の contents fragment root を新規 allocate し、
    /// その arena index を template element の `template_contents` slot に
    /// wire する。
    ///
    /// # Fragment root の shape (NodeData::DocumentFragment)
    ///
    /// Fragment root は `Document.nodes` arena に detached 状態で allocate される
    /// (parent なし、Document root からも reachable でない)。以前は
    /// `"#document-fragment"` pseudo-tag な Element として実装していたが、
    /// [`NodeData::DocumentFragment`] variant として恒久化した:
    /// - `NodeKind::DocumentFragment` として存在 (Element ではない)、
    ///   `as_element() == None` — CSS selector / cascade はそもそも Element gate
    ///   で自動 skip
    /// - `tag_name()` は `None` (pseudo-tag pollution 廃止)
    /// - `is_in_document()` は false (以下 `flags_dirty=true` →
    ///   `mark_in_document_flags` の step 1 で clear された後、step 2 で reachable
    ///   でないため false のまま)
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
    ///   element を渡すのは caller bug。
    /// - `template_id` の `template_contents` slot が既に populate されている
    ///   場合 (debug のみ)。sink は template element ごとに 1 度だけこの
    ///   method を呼ぶ契約で、二重呼び出しは古い fragment root を silently
    ///   orphan するため debug で fail。release では上書きを許容
    ///   (将来の mutation runtime での再 wire を想定した保守的挙動)。
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
        // Step 1: fragment root を detached DocumentFragment として allocate
        // (旧 append_element(None, "#document-fragment", ...)
        // pseudo-tag を廃止)。flags_dirty を明示的に set することで、後段
        // mark_in_document_flags が step 1 で default IS_IN_DOCUMENT bit を
        // clear する。
        let frag_root = self.nodes.len();
        self.nodes.push(Node::new_document_fragment());
        self.invalidate_layout_cache();
        self.flags_dirty = true;
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

    /// Element node の `inline_style` を後付けで更新する。
    /// sink が `finish()` 時に metadata table から
    /// `style="..."` を抽出して呼び出す。値は生 string でよく、`style=""`
    /// の空文字列 → `None` 正規化は Element trait 実装側
    /// ([`raikiri_traits::Element::inline_style_source`]) が行う。
    /// 二重正規化を避けるため storage 層はここで判定しない。
    ///
    /// Panics (debug + release 共通): `id` が Element kind
    /// でない場合。
    pub fn set_element_inline_style(&mut self, id: usize, inline_style: Option<SmolStr>) {
        let e = self.nodes[id]
            .data
            .as_element_mut()
            .expect("set_element_inline_style called on non-Element");
        e.inline_style = inline_style;
    }

    /// 全 node の children Vec に対して predicate を適用し、`false` を返す
    /// entry を除去する generic bulk-detach primitive。個別に
    /// `detach_from_parent` を N 回呼ぶ O(K*N) 実装を回避 (attacker-controlled
    /// な多量 mutation で quadratic を防ぐ)。tree mutation なので
    /// `invalidate_layout_cache` も call する。
    ///
    /// **Historical note**: 旧 raikiri-html sink の
    /// `strip_non_element_stubs` が Comment / PI unimplemented Element の bulk 除去に
    /// 消費していたが、Comment / ProcessingInstruction が
    /// [`NodeData`] variant として恒久 tree 保持 + `mark_in_document_flags`
    /// による IS_IN_DOCUMENT clear の 2 段 gate に置換されたため、この primitive
    /// の parse 経路での使用は無くなった。現在は将来の mutation runtime /
    /// 直接組み立てを行う consumer 向けの汎用 helper として存置。
    /// [`flags_dirty`](Self#structfield.flags_dirty) `true` を tree topology
    /// 変更時に set する契約は他 mutation primitive と一致。
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

    /// Flat tree membership bit (`IS_IN_DOCUMENT`) を全 arena node について
    /// dirty flag ベースで recompute する (Comment / PI kind への拡張を含む)。
    /// sink.finish() および mutation batch 後に呼ばれる。
    ///
    /// **どの node が clear されるか** (post-condition):
    /// - Document root から reachable でない (detached / unreachable) node
    /// - `<template>` element の子孫 (element 自身は set、その中身は clear)
    /// - `NodeData::Comment` / `NodeData::ProcessingInstruction` variant
    ///   (**reachable でも unconditionally clear** — flat tree 上 unrendered な
    ///   kind として rendering traversal から統一 skip)
    ///
    /// `NodeData::DocumentFragment` は typically detached なので step 2 の DFS
    /// が届かず step 1 の clear が残る (kind-based clear は不要)。
    ///
    /// アルゴリズム:
    /// 1. 全 arena node の bit を先に clear (detached / unreachable node を
    ///    default true のまま残さないため)
    /// 2. Document root から iterative DFS で bit set。template element 自身
    ///    は set、その descendants は skip (bit clear の状態が残る)。
    ///    Comment / PI variant は reachable でも set しない (kind gate)
    ///
    /// 補足: sink 経由の parse では template
    /// contents は fragment root subtree に流れ、Document root から reachable
    /// でなくなる → step 2 の DFS は自動的に届かない (in_template branch は
    /// 走らない)。だが本 step 2 の "template 判定 → descendants skip" logic は
    /// 残す: 手動で `append_element(Some(tmpl), ...)` を呼ぶ code path (raikiri-dom
    /// 内 test / raikiri-paint hello-world setup / 将来の mutation runtime で
    /// fragment root を経由しない contents 追加) は template 直下に子を積む
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
    /// - 本 method は taffy の effective child tree
    ///   (`TaffyChildIter` が `is_in_document()` で filter する) を変更する。
    ///   post-condition として layout cache も無効化する — さもなくば次回
    ///   `compute_child_layout` が古い child ordering で cached result を再利用
    ///   してしまう。
    /// - `flags_dirty` が false のときは no-op
    ///   (idempotent + O(1))。mutation primitive が dirty mark するため、
    ///   observation-side は毎回本 method を呼んでも overhead が amortize される。
    ///   Consumer は「mutation batch → mark → observation」の contract を守る
    ///   ことでどこかの primitive で flag 更新を忘れた場合の regression を回避
    ///   できる。
    pub fn mark_in_document_flags(&mut self) {
        // dirty check で cheap early return。
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
        //
        // Comment /
        // ProcessingInstruction は flat tree 上 unrendered なので、reachable
        // でも `IS_IN_DOCUMENT` bit は clear のままにする。これにより
        // TaffyChildIter の is_in_document filter で自動的に skip され、
        // cascade / paint / stylesheet extraction の同 filter も一貫して
        // Comment/PI を触らない (「traversal に個別の kind gate を散らさない」
        // 契約 = crates/raikiri-traits/src/dom.rs の Node::is_in_document doc)。
        // DocumentFragment は detached なので DFS が届かず、step 1 の clear
        // 状態のまま残る (追加処理不要)。
        let root = self.root_index();
        let mut stack: Vec<(usize, bool)> = vec![(root, false)];
        while let Some((id, in_template)) = stack.pop() {
            let (children_snapshot, is_template_here) = {
                let node = &mut self.nodes[id];
                let is_unrendered_by_kind = matches!(
                    node.data,
                    NodeData::Comment(_) | NodeData::ProcessingInstruction { .. }
                );
                node.set_in_document(!in_template && !is_unrendered_by_kind);
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
        // layout cache を dirty mark する。
        self.invalidate_layout_cache();
    }

    /// arena index `id` の node への借用参照。範囲外 index は `None`。
    ///
    /// blitz-dom の `BaseDocument::get_node` 相当。raikiri-paint が walk 中に
    /// per-node で呼ぶ hot path なので O(1) の `Vec::get` を wrap。
    pub fn get_node(&self, id: usize) -> Option<&Node> {
        self.nodes.get(id)
    }

    /// Return an element's namespace URI, treating an omitted HTML namespace as XHTML.
    pub fn element_namespace_uri(&self, id: usize) -> Option<&str> {
        let NodeData::Element(element) = &self.nodes.get(id)?.data else {
            return None;
        };
        Some(element.namespace.as_deref().unwrap_or(XHTML_NAMESPACE_URI))
    }

    /// Read a null-namespace attribute, applying HTML's ASCII-case-insensitive
    /// lookup rule to HTML-namespace elements only.
    pub fn element_attribute(&self, id: usize, name: &str) -> Option<&str> {
        let node = self.nodes.get(id)?;
        let NodeData::Element(element) = &node.data else {
            return None;
        };
        let local = if element
            .namespace
            .as_deref()
            .is_none_or(|namespace| namespace == XHTML_NAMESPACE_URI)
        {
            name.to_ascii_lowercase()
        } else {
            name.to_owned()
        };
        node.attribute(&local)
    }

    /// Return the post-computed Taffy style for a node.
    ///
    /// Layout mutates this style with finite used `ch` lengths before Taffy
    /// runs. Paint-side consumers can therefore observe the same used values
    /// without re-probing fonts or falling back to `ComputedValues`.
    pub fn layout_style(&self, id: usize) -> Option<&Style> {
        self.nodes.get(id).map(|node| &node.style)
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

    // ─── stylesheets ───────────────────

    /// Stylesheet を Document に associate する。
    ///
    /// - lazy: parse は行わない。cascade phase で一括処理される。
    /// - 呼び出し順で同一 `kind` 内の cascade source_order が決まる。
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

    // ─── quirks mode ───────────────────

    /// この Document の HTML5 quirks mode を設定する。raikiri-html の parse
    /// sink が html5ever `TreeSink::set_quirks_mode` callback で得た値を
    /// `finish()` 時にここへ書き込む想定 (`RaikiriTreeSink::finish` 参照)。
    pub fn set_quirks_mode(&mut self, mode: QuirksMode) {
        self.quirks_mode = mode;
    }

    /// この Document の HTML5 quirks mode。手動構築された `Document` (parse
    /// を経由しない test / setup コード) では [`QuirksMode::NoQuirks`]
    /// のまま。`impl raikiri_style::StyleDom for Document` の
    /// `quirks_mode()` override がこの値を `StyleQuirksMode` へ変換して
    /// cascade に渡す (`dom_impl.rs`)。
    pub fn quirks_mode(&self) -> QuirksMode {
        self.quirks_mode
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
        // Regression check: arena に存在するが
        // Document root から reachable でない node (foster-parenting transient
        // state / unimplemented-node 除去後の孤児 等) は mark 後 is_in_document=false に落ちる
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
        // Regression check for the existing contract: template element
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
        // Regression check: mutation primitives が flags_dirty を
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
        // mark_in_document_flags は !flags_dirty のとき
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
        // Regression check: `set_element_namespace` は
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
        // Regression check: mutation → 再 mark で正しい bit 状態が復元
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
    // Regression check: find_body (both layout and paint impls)
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
    use crate::node::NodeFlags;
    use taffy::TraversePartialTree;

    #[test]
    fn taffy_child_ids_and_count_filter_out_template_descendants() {
        // Regression pin。taffy layout tree
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

    #[test]
    fn synthetic_inline_root_keeps_collapsed_whitespace_child() {
        let mut doc = Document::new();
        let root = doc.root_index();
        let parent = doc.append_element(Some(root), "div", Style::default(), None::<&str>);
        let whitespace = doc.append_text(parent, "\n  ");
        doc.nodes[parent].style.display = taffy::Display::Flex;
        doc.mark_in_document_flags();

        let parent_id = taffy::NodeId::from(parent);
        assert_eq!(
            <Document as TraversePartialTree>::child_count(&doc, parent_id),
            0,
        );

        doc.nodes[parent].flags.insert(NodeFlags::IS_INLINE_ROOT);
        let children: Vec<taffy::NodeId> =
            <Document as TraversePartialTree>::child_ids(&doc, parent_id).collect();
        assert_eq!(children, vec![taffy::NodeId::from(whitespace)]);
    }

    #[test]
    fn taffy_child_ids_and_count_filter_out_comment_and_pi_variants() {
        // Regression check:
        // Comment / ProcessingInstruction variant を body 直下に attach した後
        // mark_in_document_flags を経由すると、TaffyChildIter は
        // is_in_document filter でこれらを skip する。旧 strip_non_element_stubs
        // が担っていた "layout tree から non-Element node を消す" 機能が、
        // strip 廃止後は「NodeData variant → mark_in_document_flags で
        // IS_IN_DOCUMENT clear → TaffyChildIter が filter」の chain に置き換わって
        // いることを end-to-end で pin。
        //
        // 特に「Comment/PI が layout child count に leak する」
        // failure mode を stress する: body 直下に Comment 3 個 + PI 2 個 + <p>、
        // という mix で、body の taffy child_count == 1 (<p> only) を要求する。
        let mut doc = Document::new();
        let root = doc.root_index();
        let body = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
        // Interleaved で attach、ordering に依存しないことを確認。
        let _c0 = doc.append_comment(Some(body), "hello");
        let _pi0 = doc.append_processing_instruction(Some(body), "xml-stylesheet", "href='x'");
        let _c1 = doc.append_comment(Some(body), "middle");
        let p = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
        let _c2 = doc.append_comment(Some(body), "end");
        let _pi1 = doc.append_processing_instruction(Some(body), "xml", "version='1.0'");

        doc.mark_in_document_flags();

        // (a) Comment / PI variant node は IS_IN_DOCUMENT が clear されている。
        for i in 0..doc.node_count() {
            let n = doc.get_node(i).unwrap();
            match n.kind() {
                raikiri_traits::NodeKind::Comment
                | raikiri_traits::NodeKind::ProcessingInstruction => {
                    assert!(
                        !n.is_in_document(),
                        "Comment/PI at arena idx {i} must have IS_IN_DOCUMENT cleared \
                         after mark_in_document_flags"
                    );
                }
                _ => {}
            }
        }

        // (b) taffy layout tree から見た body の child は <p> の 1 個のみ。
        let body_taffy = taffy::NodeId::from(body);
        let p_taffy = taffy::NodeId::from(p);
        assert_eq!(
            <Document as TraversePartialTree>::child_count(&doc, body_taffy),
            1,
            "body's taffy child_count must be 1 (only <p>), Comment/PI filtered"
        );
        let kids: Vec<taffy::NodeId> =
            <Document as TraversePartialTree>::child_ids(&doc, body_taffy).collect();
        assert_eq!(kids, vec![p_taffy]);

        // (c) get_child_id も filtered view で consistent (index 0 = <p>)。
        assert_eq!(
            <Document as TraversePartialTree>::get_child_id(&doc, body_taffy, 0),
            p_taffy
        );
    }
}

#[cfg(test)]
mod attach_child_fragment_tests {
    //! attach_child が
    //! `NodeData::DocumentFragment` を child に受け取った時、WHATWG DOM §4.2.3
    //! Mutation algorithms — insert algorithm steps 1 + 4.1 + 7.2 と一致する
    //! fragment-aware semantics で動作する契約を check (append が positional
    //! splice の step 7.3 ではなく 7.2 に対応する導出は `Document::attach_child`
    //! の doc comment 参照)。
    //!
    //! Spec ref:
    //! - <https://dom.spec.whatwg.org/#concept-node-pre-insert>
    //! - <https://dom.spec.whatwg.org/#concept-node-insert>
    //!
    //! 契約 (test 5 分割):
    //! (a) parent.children が fragment の children で source order に extend される
    //! (b) fragment の children Vec が empty 化される (move、not clone)
    //! (c) fragment node 自身は parent.children に含まれない
    //! (d) empty fragment attach は parent.children を変えない (edge)
    //! (e) fragment 以外の child は旧 push 挙動を維持する (regression check)
    use super::*;
    use crate::node::NodeData;

    fn make_fragment_with_two_children(doc: &mut Document) -> (usize, usize, usize) {
        let frag = doc.nodes.len();
        doc.nodes.push(Node::new_document_fragment());
        // fragment の children は detached の Element 2 個。
        let c0 = doc.append_element(Some(frag), "span", Style::default(), None::<&str>);
        let c1 = doc.append_element(Some(frag), "div", Style::default(), None::<&str>);
        (frag, c0, c1)
    }

    #[test]
    fn attach_child_extends_parent_with_fragment_children_in_order() {
        // WHATWG DOM §4.2.3 Mutation algorithms — insert steps 1 + 7.2 with a
        // fragment child: node's children → parent's children, in tree order.
        let mut doc = Document::new();
        let root = doc.root_index();
        let parent = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
        // parent には先に既存 child を 1 個入れておく。
        let pre_existing = doc.append_element(Some(parent), "pre", Style::default(), None::<&str>);

        let (frag, c0, c1) = make_fragment_with_two_children(&mut doc);
        doc.attach_child(parent, frag);

        // (a): parent.children == `[pre_existing, c0, c1]` (append at tail, source order)。
        assert_eq!(
            doc.nodes[parent].children,
            vec![pre_existing, c0, c1],
            "attach_child with DocumentFragment must extend parent's children with fragment's children in source order"
        );
    }

    #[test]
    fn attach_child_empties_fragments_children_after_move() {
        // (b): fragment の children Vec は空になる (move semantics、clone ではない)。
        // 旧 push 挙動なら fragment.children は保たれるので、この test が move を check する。
        let mut doc = Document::new();
        let root = doc.root_index();
        let parent = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
        let (frag, _c0, _c1) = make_fragment_with_two_children(&mut doc);

        // 事前確認: fragment 自身は 2 個の children を持つ。
        assert_eq!(doc.nodes[frag].children.len(), 2);

        doc.attach_child(parent, frag);
        assert!(
            doc.nodes[frag].children.is_empty(),
            "fragment's children must be drained after attach_child (move semantics per WHATWG DOM §4.2.3 Mutation algorithms — insert step 4.1)"
        );
        // fragment 自身は arena には残る (kind = DocumentFragment、detached)。
        assert!(matches!(doc.nodes[frag].data, NodeData::DocumentFragment));
    }

    #[test]
    fn attach_child_does_not_push_the_fragment_node_itself() {
        // (c): fragment node 自身は parent.children に絶対に含まれない。
        // WHATWG DOM §4.2.3 Mutation algorithms — insert step 1 は fragment の
        // 場合 nodes = fragment.children と定義し、fragment 自身は tree
        // insertion 対象外となる (mutation record 上も
        // parent → fragment ではなく parent → fragment's children で観測される)。
        let mut doc = Document::new();
        let root = doc.root_index();
        let parent = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
        let (frag, _c0, _c1) = make_fragment_with_two_children(&mut doc);

        doc.attach_child(parent, frag);

        assert!(
            !doc.nodes[parent].children.contains(&frag),
            "fragment node itself must NOT appear in parent.children (spec: fragment is unrendered container)"
        );
    }

    #[test]
    fn attach_child_with_empty_fragment_is_noop_on_parent_children() {
        // (d): empty fragment attach は parent の children を変えない。
        // security lens (raw-arena-index footgun): empty fragment で
        // drain().collect() が空 Vec を返し extend が何もしないことを pin。
        let mut doc = Document::new();
        let root = doc.root_index();
        let parent = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
        let existing = doc.append_element(Some(parent), "p", Style::default(), None::<&str>);

        // Empty fragment。
        let empty_frag = doc.nodes.len();
        doc.nodes.push(Node::new_document_fragment());

        doc.attach_child(parent, empty_frag);
        assert_eq!(
            doc.nodes[parent].children,
            vec![existing],
            "empty fragment attach must not change parent.children"
        );
        assert!(!doc.nodes[parent].children.contains(&empty_frag));
    }

    #[test]
    fn attach_child_non_fragment_keeps_existing_push_semantics() {
        // (e): fragment 以外 (Element / Text / Comment / PI /
        // Document) は旧 挙動 (単純 push) 継続、fragment 分岐が collateral damage
        // を出さないことを pin。
        let mut doc = Document::new();
        let root = doc.root_index();
        let parent = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
        // Element child, detached。
        let elem = doc.append_element(None, "span", Style::default(), None::<&str>);
        doc.attach_child(parent, elem);
        assert_eq!(doc.nodes[parent].children, vec![elem]);
        // Comment child, detached。
        let comment = doc.append_comment(None, "hi");
        doc.attach_child(parent, comment);
        assert_eq!(doc.nodes[parent].children, vec![elem, comment]);
    }
}

#[cfg(test)]
mod insert_child_before_fragment_tests {
    //! `insert_child_before` が
    //! [`NodeData::DocumentFragment`] を child に受け取った時、fragment の
    //! children を parent.children の `before` position から source order で
    //! splice する fragment-aware semantics (WHATWG DOM §4.2.3 Mutation
    //! algorithms — insert algorithm steps 1 + 4.1 + 7.3) を pin。
    //! attach_child (tail append) の positional 対応で、これまで latent
    //! だった asymmetry を解消する。
    //!
    //! Spec ref:
    //! - <https://dom.spec.whatwg.org/#concept-node-pre-insert>
    //! - <https://dom.spec.whatwg.org/#concept-node-insert>
    //!
    //! 契約 (test 5 分割 = `attach_child_fragment_tests` の mirror):
    //! (a) parent.children が fragment の children で `before` position から
    //!     source order で splice される
    //! (b) fragment の children Vec が empty 化される (move、not clone)
    //! (c) fragment node 自身は parent.children に含まれない
    //! (d) empty fragment splice は parent.children を変えない (edge)
    //! (e) fragment 以外の child は旧 insert 挙動を維持する (regression check)
    use super::*;
    use crate::node::NodeData;

    fn make_fragment_with_two_children(doc: &mut Document) -> (usize, usize, usize) {
        let frag = doc.nodes.len();
        doc.nodes.push(Node::new_document_fragment());
        // fragment の children は detached の Element 2 個。
        let c0 = doc.append_element(Some(frag), "span", Style::default(), None::<&str>);
        let c1 = doc.append_element(Some(frag), "div", Style::default(), None::<&str>);
        (frag, c0, c1)
    }

    #[test]
    fn insert_child_before_splices_fragment_children_at_position() {
        // (a): parent.children の `before` position に fragment の children が
        // source order で挿入される。tail append の attach_child と違い、
        // positional な splice を check する (`before` の直前に fragment children
        // 全部、その後 `before` 自体、以降既存 sibling が続く)。
        let mut doc = Document::new();
        let root = doc.root_index();
        let parent = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
        // parent には既に 2 個 sibling がある: `[first, last]`。
        let first = doc.append_element(Some(parent), "h1", Style::default(), None::<&str>);
        let last = doc.append_element(Some(parent), "footer", Style::default(), None::<&str>);

        let (frag, c0, c1) = make_fragment_with_two_children(&mut doc);
        doc.insert_child_before(parent, last, frag);

        // fragment children c0, c1 が last の前 (= first と last の間) に挿入される。
        assert_eq!(
            doc.nodes[parent].children,
            vec![first, c0, c1, last],
            "insert_child_before with DocumentFragment must splice fragment's children at the `before` position in source order"
        );
    }

    #[test]
    fn insert_child_before_empties_fragments_children_after_move() {
        // (b): fragment の children Vec は空になる (move semantics、clone ではない)。
        // 旧挙動 (fragment 自身を単純 insert) なら fragment.children は保たれる
        // ので、この test が drain の move semantics を check する。
        let mut doc = Document::new();
        let root = doc.root_index();
        let parent = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
        let sibling = doc.append_element(Some(parent), "p", Style::default(), None::<&str>);

        let (frag, _c0, _c1) = make_fragment_with_two_children(&mut doc);
        // 事前確認: fragment 自身は 2 個の children を持つ。
        assert_eq!(doc.nodes[frag].children.len(), 2);

        doc.insert_child_before(parent, sibling, frag);
        assert!(
            doc.nodes[frag].children.is_empty(),
            "fragment's children must be drained after insert_child_before (move semantics per WHATWG DOM §4.2.3 Mutation algorithms — insert step 4.1)"
        );
        // fragment 自身は arena には残る (kind = DocumentFragment、detached)。
        assert!(matches!(doc.nodes[frag].data, NodeData::DocumentFragment));
    }

    #[test]
    fn insert_child_before_does_not_insert_the_fragment_node_itself() {
        // (c): fragment node 自身は parent.children に絶対に含まれない。
        // WHATWG DOM §4.2.3 Mutation algorithms — insert step 1 は fragment の場合
        // nodes = fragment.children と定義し、fragment 自身は tree insertion 対象外
        // となる (mutation record 上も parent → fragment ではなく
        // parent → fragment's children で観測される)。
        let mut doc = Document::new();
        let root = doc.root_index();
        let parent = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
        let sibling = doc.append_element(Some(parent), "p", Style::default(), None::<&str>);

        let (frag, _c0, _c1) = make_fragment_with_two_children(&mut doc);
        doc.insert_child_before(parent, sibling, frag);

        assert!(
            !doc.nodes[parent].children.contains(&frag),
            "fragment node itself must NOT appear in parent.children (spec: fragment is unrendered container)"
        );
    }

    #[test]
    fn insert_child_before_with_empty_fragment_is_noop_on_parent_children() {
        // (d): empty fragment splice は parent の children を変えない。
        // splice(pos..pos, empty_vec) が何もしないことを check (attach_child edge
        // check の positional 対応)。
        let mut doc = Document::new();
        let root = doc.root_index();
        let parent = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
        let a = doc.append_element(Some(parent), "a", Style::default(), None::<&str>);
        let b = doc.append_element(Some(parent), "b", Style::default(), None::<&str>);

        // Empty fragment。
        let empty_frag = doc.nodes.len();
        doc.nodes.push(Node::new_document_fragment());

        doc.insert_child_before(parent, b, empty_frag);
        assert_eq!(
            doc.nodes[parent].children,
            vec![a, b],
            "empty fragment insert_child_before must not change parent.children"
        );
        assert!(!doc.nodes[parent].children.contains(&empty_frag));
    }

    #[test]
    fn insert_child_before_non_fragment_keeps_existing_insert_semantics() {
        // (e) (acceptance 4): fragment 以外 (Element / Text / Comment
        // / PI / Document) は旧挙動 (単純 insert at `before` position) 継続、
        // fragment 分岐が collateral damage を出さないことを pin。
        let mut doc = Document::new();
        let root = doc.root_index();
        let parent = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
        let last = doc.append_element(Some(parent), "z", Style::default(), None::<&str>);
        // Detached Element を before=last で insert (foster parenting の primitive)。
        let elem = doc.append_element(None, "m", Style::default(), None::<&str>);
        doc.insert_child_before(parent, last, elem);
        assert_eq!(doc.nodes[parent].children, vec![elem, last]);
    }
}

#[cfg(test)]
mod element_attribute_mutation_tests {
    use super::*;
    use raikiri_traits::{Dom as _, Element as _, Node as _, NodeId};
    use taffy::Style;

    fn element_attributes(doc: &Document, id: usize) -> Vec<(String, String)> {
        let NodeData::Element(element) = &doc.nodes[id].data else {
            panic!("expected element node"); // cov:ignore: test helper callers construct this node as an Element.
        };
        element
            .attributes
            .iter()
            .map(|attr| (attr.local.to_string(), attr.value.to_string()))
            .collect()
    }

    #[test]
    fn xml_name_validation_accepts_the_non_ascii_name_ranges() {
        for ch in [
            'À', 'Ø', 'ø', 'Ͱ', 'Ϳ', '\u{200c}', '⁰', 'Ⰰ', '々', '豈', 'ﷰ', '𐀀',
        ] {
            assert!(is_xml_name_start(ch));
        }
        for ch in ['0', '-', '.', '·', '\u{0300}', '\u{203f}'] {
            assert!(is_xml_name_char(ch));
        }
        assert!(is_valid_xml_name("π\u{0300}:name"));
        assert!(!is_valid_xml_name("0name"));
    }

    #[test]
    #[should_panic(expected = "set_element_attribute called on non-Element")]
    fn set_element_attribute_panics_for_a_non_element() {
        let mut doc = Document::new();
        let root = doc.root_index();
        let _ = doc.set_element_attribute(root, "id", "not-an-element");
    }

    #[test]
    #[should_panic(expected = "remove_element_attribute called on non-Element")]
    fn remove_element_attribute_panics_for_a_non_element() {
        let mut doc = Document::new();
        let root = doc.root_index();
        let _ = doc.remove_element_attribute(root, "id");
    }

    #[test]
    fn remove_element_attribute_validates_names_and_preserves_foreign_case() {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
        let svg = doc.append_element(Some(0), "svg", Style::default(), None::<&str>);
        doc.set_element_namespace(svg, Some(SmolStr::new("http://www.w3.org/2000/svg")));

        assert!(doc.remove_element_attribute(html, "bad name").is_err());
        doc.set_element_attribute(svg, "viewBox", "0 0 10 10")
            .unwrap();
        assert_eq!(doc.remove_element_attribute(svg, "viewbox").unwrap(), None);
        assert_eq!(
            doc.remove_element_attribute(svg, "viewBox").unwrap(),
            Some(SmolStr::new("0 0 10 10"))
        );
    }

    #[test]
    fn element_attribute_accessors_return_none_for_non_elements_and_match_html_case() {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
        let text = doc.append_text(html, "text");
        doc.set_element_attribute(html, "title", "heading").unwrap();
        assert_eq!(doc.element_namespace_uri(html), Some(XHTML_NAMESPACE_URI));
        assert_eq!(doc.element_namespace_uri(text), None);
        assert_eq!(doc.element_namespace_uri(usize::MAX), None);
        assert_eq!(doc.element_attribute(html, "TITLE"), Some("heading"));
        assert_eq!(doc.element_attribute(text, "title"), None);
        assert_eq!(doc.element_attribute(usize::MAX, "title"), None);
    }

    #[test]
    fn set_and_remove_element_attribute_preserve_order_and_route_style_separately() {
        let mut doc = Document::new();
        let id = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
        doc.set_element_attributes(
            id,
            vec![
                (SmolStr::new("id"), SmolStr::new("old")),
                (SmolStr::new("data-empty"), SmolStr::new("")),
                (SmolStr::new("id"), SmolStr::new("duplicate")),
            ],
        );

        doc.set_element_attribute(id, "id", "new").unwrap();
        doc.set_element_attribute(id, "title", "heading").unwrap();
        assert_eq!(
            element_attributes(&doc, id),
            vec![
                ("id".into(), "new".into()),
                ("data-empty".into(), "".into()),
                ("title".into(), "heading".into()),
            ],
            "updating an attribute keeps its position and a new attribute appends" // cov:ignore: assert_eq! formats this diagnostic only on failure.
        );

        doc.set_element_attribute(id, "style", "color: red")
            .unwrap();
        assert_eq!(
            element_attributes(&doc, id),
            vec![
                ("id".into(), "new".into()),
                ("data-empty".into(), "".into()),
                ("title".into(), "heading".into()),
            ],
            "style must not enter the null-namespace attribute list" // cov:ignore: assert_eq! formats this diagnostic only on failure.
        );
        let NodeData::Element(element) = &doc.nodes[id].data else {
            unreachable!(); // cov:ignore: the setup above creates this node as an Element.
        };
        assert_eq!(element.inline_style.as_deref(), Some("color: red"));

        let node = doc.node(NodeId::new(id as u64)).expect("element exists");
        let element = node.as_element().expect("node is an element");
        assert_eq!(element.attr("id"), Some("new"));
        assert_eq!(element.attr("style"), Some("color: red"));

        assert_eq!(
            doc.remove_element_attribute(id, "data-empty").unwrap(),
            Some(SmolStr::new(""))
        );
        assert_eq!(
            doc.remove_element_attribute(id, "style").unwrap(),
            Some(SmolStr::new("color: red"))
        );
        assert_eq!(doc.remove_element_attribute(id, "missing").unwrap(), None);
        assert_eq!(
            element_attributes(&doc, id),
            vec![
                ("id".into(), "new".into()),
                ("title".into(), "heading".into()),
            ]
        );
        let NodeData::Element(element) = &doc.nodes[id].data else {
            unreachable!(); // cov:ignore: the setup above creates this node as an Element.
        };
        assert_eq!(element.inline_style, None);
    }
}

#[cfg(test)]
mod replace_children_from_tests {
    use super::*;
    use crate::node::NodeFlags;
    use taffy::Style;

    #[test]
    fn replace_children_from_deep_copies_nodes_in_order_and_detaches_old_subtree() {
        let mut target = Document::new();
        let target_parent = target.append_element(Some(0), "main", Style::default(), None::<&str>);
        let old = target.append_element(Some(target_parent), "old", Style::default(), None::<&str>);
        let old_text = target.append_text(old, "kept in arena");

        let mut source = Document::new();
        let source_parent =
            source.append_element(Some(0), "source", Style::default(), None::<&str>);
        let source_comment = source.append_comment(Some(source_parent), "leading comment");
        let source_instruction =
            source.append_processing_instruction(Some(source_parent), "work", "ready");
        let section = source.append_element(
            Some(source_parent),
            "section",
            Style::default(),
            Some("color: red"),
        );
        source.set_element_namespace(section, Some(SmolStr::new("urn:example")));
        source.set_element_attributes(
            section,
            vec![
                (SmolStr::new("id"), SmolStr::new("copied")),
                (SmolStr::new("data-key"), SmolStr::new("value")),
            ],
        );
        let text = source.append_text(section, "first");
        let comment = source.append_comment(Some(section), "middle comment");
        let span = source.append_element(Some(section), "span", Style::default(), None::<&str>);
        source.set_element_attributes(span, vec![(SmolStr::new("title"), SmolStr::new("tip"))]);
        source
            .set_element_attribute(span, "style", "display: block")
            .unwrap();
        let trailing_text = source.append_text(section, "last");

        // Reset the target flags to prove this operation routes mutations
        // through the normal invalidation paths.
        target.layout_dirty = false;
        target.flags_dirty = false;
        target.replace_children_from(target_parent, &source, source_parent);

        assert!(target.layout_dirty);
        assert!(target.flags_dirty);
        assert_eq!(target.nodes[target_parent].children.len(), 3);
        let copied_comment = target.nodes[target_parent].children[0];
        let copied_instruction = target.nodes[target_parent].children[1];
        let copied_section = target.nodes[target_parent].children[2];
        assert_ne!(copied_comment, source_comment);
        assert_ne!(copied_instruction, source_instruction);
        assert_ne!(copied_section, section);
        assert!(matches!(
            target.nodes[copied_comment].data,
            NodeData::Comment(_)
        ));
        assert!(matches!(
            &target.nodes[copied_instruction].data,
            NodeData::ProcessingInstruction { target, data }
                if target == "work" && data == "ready"
        ));
        assert_eq!(
            target.parent_of(old),
            None,
            "replaced nodes stay allocated but detached" // cov:ignore: assert_eq! formats this diagnostic only on failure.
        );
        assert_eq!(
            target.nodes[old].children,
            vec![old_text],
            "the old subtree remains allocated" // cov:ignore: assert_eq! formats this diagnostic only on failure.
        );
        assert_eq!(target.parent_of(old_text), Some(old));

        let NodeData::Element(copied_element) = &target.nodes[copied_section].data else {
            panic!("expected copied section element"); // cov:ignore: the source fixture constructs this as an Element.
        };
        assert_eq!(copied_element.tag_name.as_str(), "section");
        assert_eq!(copied_element.namespace.as_deref(), Some("urn:example"));
        assert_eq!(copied_element.inline_style.as_deref(), Some("color: red"));
        assert_eq!(
            copied_element
                .attributes
                .iter()
                .map(|attr| (attr.local.as_str(), attr.value.as_str()))
                .collect::<Vec<_>>(),
            vec![("id", "copied"), ("data-key", "value")]
        );

        let copied_children = target.nodes[copied_section].children.clone();
        assert_eq!(copied_children.len(), 4);
        let copied_text = copied_children[0];
        let copied_comment = copied_children[1];
        let copied_span = copied_children[2];
        let copied_trailing_text = copied_children[3];
        assert_ne!(copied_text, text);
        assert_ne!(copied_comment, comment);
        assert_ne!(copied_trailing_text, trailing_text);
        assert!(
            matches!(&target.nodes[copied_text].data, NodeData::Text(t) if t.text_content == "first")
        );
        assert!(
            matches!(&target.nodes[copied_comment].data, NodeData::Comment(t) if t == "middle comment")
        );
        assert!(
            matches!(&target.nodes[copied_trailing_text].data, NodeData::Text(t) if t.text_content == "last")
        );
        let NodeData::Element(copied_span_data) = &target.nodes[copied_span].data else {
            panic!("expected copied span element"); // cov:ignore: the source fixture constructs this as an Element.
        };
        assert_eq!(
            copied_span_data.inline_style.as_deref(),
            Some("display: block")
        );
        assert_eq!(copied_span_data.attributes.len(), 1);
        assert_eq!(copied_span_data.attributes[0].local.as_str(), "title");
        assert_eq!(copied_span_data.attributes[0].value.as_str(), "tip");

        target.mark_in_document_flags();
        assert!(!target.nodes[old].flags.contains(NodeFlags::IS_IN_DOCUMENT));
        assert!(
            !target.nodes[old_text]
                .flags
                .contains(NodeFlags::IS_IN_DOCUMENT)
        );
        assert!(
            target.nodes[copied_section]
                .flags
                .contains(NodeFlags::IS_IN_DOCUMENT)
        );
        assert!(
            !target.nodes[copied_comment]
                .flags
                .contains(NodeFlags::IS_IN_DOCUMENT)
        );
        // Copying does not mutate or consume source children.
        assert_eq!(
            source.nodes[source_parent].children,
            vec![source_comment, source_instruction, section]
        );
    }

    #[test]
    fn replace_children_from_splices_document_fragment_children() {
        let mut source = Document::new();
        let source_parent =
            source.append_element(Some(0), "section", Style::default(), None::<&str>);
        let fragment = source.nodes.len();
        source.nodes.push(Node::new_document_fragment());
        source.nodes[source_parent].children.push(fragment);
        source.append_text(fragment, "fragment text");

        let mut target = Document::new();
        let target_parent = target.append_element(Some(0), "main", Style::default(), None::<&str>);
        target.replace_children_from(target_parent, &source, source_parent);
        assert_eq!(
            target.serialize_inner_html(target_parent).unwrap(),
            "fragment text"
        );
    }

    #[test]
    #[should_panic(expected = "replace_children_from cannot copy a Document node as a child")]
    fn replace_children_from_rejects_document_nodes_in_child_lists() {
        let mut source = Document::new();
        let source_parent =
            source.append_element(Some(0), "section", Style::default(), None::<&str>);
        let source_root = source.root_index();
        source.nodes[source_parent].children.push(source_root);

        let mut target = Document::new();
        let target_parent = target.append_element(Some(0), "main", Style::default(), None::<&str>);
        target.replace_children_from(target_parent, &source, source_parent);
    }
}

#[cfg(test)]
mod serialize_inner_html_tests {
    use super::*;
    use taffy::Style;

    #[test]
    fn serialize_inner_html_uses_live_attributes_escapes_content_and_omits_void_end_tags() {
        let mut doc = Document::new();
        let host = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
        let paragraph = doc.append_element(Some(host), "p", Style::default(), None::<&str>);
        doc.set_element_attributes(
            paragraph,
            vec![(SmolStr::new("data-empty"), SmolStr::new(""))],
        );
        doc.set_element_attribute(paragraph, "title", "old")
            .unwrap();
        doc.set_element_attribute(paragraph, "title", "a&b\"c'd")
            .unwrap();
        doc.set_element_attribute(paragraph, "style", "color: red")
            .unwrap();
        doc.set_element_attributes(
            paragraph,
            vec![
                (SmolStr::new("data-empty"), SmolStr::new("")),
                (SmolStr::new("title"), SmolStr::new("a&b\"c'd")),
                (SmolStr::new("style"), SmolStr::new("legacy style entry")),
            ],
        );
        let text = doc.append_text(paragraph, "A < B & C > D");
        doc.append_comment(Some(paragraph), "note");
        doc.append_processing_instruction(Some(paragraph), "target", "data");
        doc.append_element(Some(paragraph), "br", Style::default(), None::<&str>);

        assert_eq!(
            doc.serialize_inner_html(host).unwrap(),
            "<p data-empty=\"\" title=\"a&amp;b&quot;c&#39;d\" style=\"color: red\">A &lt; B &amp; C &gt; D<!--note--><?target data?><br></p>",
            "serialize attributes, text, comments, processing instructions, and void elements in order" // cov:ignore: assert_eq! formats this diagnostic only on failure.
        );
        assert!(doc.serialize_inner_html(doc.nodes.len()).is_err());
        assert!(doc.serialize_inner_html(text).is_err());
        let serialized_document = doc.serialize_inner_html(doc.root_index()).unwrap();
        assert!(serialized_document.starts_with("<div>"));
        assert!(serialized_document.ends_with("</div>"));
    }

    #[test]
    fn serializes_raw_text_and_rejects_invalid_attribute_names() {
        let mut doc = Document::new();
        let host = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
        let script = doc.append_element(Some(host), "script", Style::default(), None::<&str>);
        doc.append_text(script, "if (a < b && c > d) {}");

        assert_eq!(
            doc.serialize_inner_html(script).unwrap(),
            "if (a < b && c > d) {}"
        );
        assert_eq!(
            doc.serialize_inner_html(host).unwrap(),
            "<script>if (a < b && c > d) {}</script>"
        );
        assert!(
            doc.set_element_attribute(host, "x\" onmouseover=\"bad", "1")
                .is_err()
        );
        assert_eq!(
            doc.serialize_inner_html(host).unwrap(),
            "<script>if (a < b && c > d) {}</script>"
        );
        let malformed = doc.append_element(Some(host), "span", Style::default(), None::<&str>);
        doc.set_element_attributes(
            malformed,
            vec![(SmolStr::new("bad name"), SmolStr::new("value"))],
        );
        assert!(
            doc.serialize_inner_html(host)
                .unwrap_err()
                .contains("invalid attribute name")
        );
    }

    #[test]
    fn attribute_names_follow_html_and_foreign_content_case_rules() {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
        doc.set_element_attribute(html, "DATA-Key", "one").unwrap();
        doc.set_element_attribute(html, "data-key", "two").unwrap();
        assert_eq!(doc.element_attribute(html, "DATA-KEY"), Some("two"));

        let svg = doc.append_element(Some(0), "svg", Style::default(), None::<&str>);
        doc.set_element_namespace(svg, Some("http://www.w3.org/2000/svg".into()));
        doc.set_element_attribute(svg, "viewBox", "0 0 10 10")
            .unwrap();
        doc.set_element_attribute(svg, "viewbox", "lowercase")
            .unwrap();
        assert_eq!(doc.element_attribute(svg, "viewBox"), Some("0 0 10 10"));
        assert_eq!(doc.element_attribute(svg, "viewbox"), Some("lowercase"));
        assert_eq!(doc.element_attribute(svg, "VIEWBOX"), None);
    }

    #[test]
    fn replace_children_from_targets_template_contents() {
        let mut target = Document::new();
        let template = target.append_element(Some(0), "template", Style::default(), None::<&str>);
        let template_contents = target.allocate_template_fragment_root(template);
        target.append_text(template_contents, "old");

        let mut source = Document::new();
        let span = source.append_element(Some(0), "span", Style::default(), None::<&str>);
        source.append_text(span, "new");

        target.replace_children_from(template, &source, source.root_index());
        assert!(target.nodes[template].children.is_empty());
        assert_eq!(target.nodes[template_contents].children.len(), 1);
        assert_eq!(
            target.serialize_inner_html(template).unwrap(),
            "<span>new</span>"
        );
    }

    #[test]
    fn serialize_inner_html_reads_template_contents_fragment() {
        let mut doc = Document::new();
        let host = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
        let template = doc.append_element(Some(host), "template", Style::default(), None::<&str>);
        let contents = doc.allocate_template_fragment_root(template);
        doc.append_text(contents, "<template text>");

        assert_eq!(
            doc.serialize_inner_html(template).unwrap(),
            "&lt;template text&gt;"
        );
        assert_eq!(
            doc.serialize_inner_html(host).unwrap(),
            "<template>&lt;template text&gt;</template>"
        );
        assert_eq!(
            doc.serialize_inner_html(contents).unwrap(),
            "&lt;template text&gt;"
        );
    }

    #[test]
    fn serialize_inner_html_reports_malformed_template_fragment_links() {
        let mut doc = Document::new();
        let host = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
        let parent_template =
            doc.append_element(Some(0), "template", Style::default(), None::<&str>);
        let nested_template =
            doc.append_element(Some(host), "template", Style::default(), None::<&str>);
        let document_root = doc.root_index();
        let wrong_kind = doc.append_text(document_root, "not a fragment");

        doc.nodes[parent_template]
            .data
            .as_element_mut()
            .unwrap()
            .template_contents = Some(wrong_kind);
        assert!(
            doc.serialize_inner_html(parent_template)
                .unwrap_err()
                .contains("not a fragment")
        );
        doc.nodes[parent_template]
            .data
            .as_element_mut()
            .unwrap()
            .template_contents = Some(usize::MAX);
        assert!(
            doc.serialize_inner_html(parent_template)
                .unwrap_err()
                .contains("out of range")
        );

        doc.nodes[nested_template]
            .data
            .as_element_mut()
            .unwrap()
            .template_contents = Some(usize::MAX);
        assert!(
            doc.serialize_inner_html(host)
                .unwrap_err()
                .contains("out of range")
        );
        doc.nodes[nested_template]
            .data
            .as_element_mut()
            .unwrap()
            .template_contents = Some(wrong_kind);
        assert!(
            doc.serialize_inner_html(host)
                .unwrap_err()
                .contains("not a fragment")
        );
    }

    #[test]
    fn serialize_inner_html_flattens_fragment_children_and_rejects_document_children() {
        let mut doc = Document::new();
        let host = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
        let fragment = doc.nodes.len();
        doc.nodes.push(Node::new_document_fragment());
        doc.nodes[host].children.push(fragment);
        doc.append_text(fragment, "fragment child");
        assert_eq!(doc.serialize_inner_html(host).unwrap(), "fragment child");

        let document_root = doc.root_index();
        doc.nodes[host].children.push(document_root);
        assert!(
            doc.serialize_inner_html(host)
                .unwrap_err()
                .contains("Document node")
        );
    }
}
