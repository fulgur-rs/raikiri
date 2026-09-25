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
use std::collections::HashMap;
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

fn qualified_name(prefix: Option<&str>, local: &str) -> String {
    prefix.map_or_else(|| local.to_owned(), |prefix| format!("{prefix}:{local}"))
}

fn push_xml_escaped(output: &mut String, value: &str, attribute: bool) {
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' if attribute => output.push_str("&quot;"),
            '\'' if attribute => output.push_str("&apos;"),
            '\n' if attribute => output.push_str("&#xA;"),
            '\r' if attribute => output.push_str("&#xD;"),
            '\t' if attribute => output.push_str("&#x9;"),
            character => output.push(character),
        }
    }
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
    /// [`LAYOUT_WARN_CAP`]: crate::layout::sanitize::LAYOUT_WARN_CAP
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

    /// Create an HTML element without attaching it to a parent.
    pub fn create_detached_element(&mut self, tag: &str) -> Result<usize, String> {
        if !is_valid_xml_name(tag) {
            return Err(format!("invalid HTML element name: {tag:?}"));
        }
        Ok(self.append_element(None, tag, Style::default(), None::<&str>))
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

    /// Append a detached or already-connected child to an element using DOM move semantics.
    ///
    /// The child is detached from its current parent before it is appended. This
    /// also rejects cycles; low-level parser operations should keep using
    /// [`Document::attach_child`] and detach explicitly as required by TreeSink.
    pub fn append_child(&mut self, parent: usize, child: usize) -> Result<(), String> {
        let Some(parent_node) = self.nodes.get(parent) else {
            return Err(format!("appendChild parent index {parent} is out of range"));
        };
        if !matches!(&parent_node.data, NodeData::Element(_)) {
            return Err("appendChild parent must be an Element".into());
        }
        let Some(child_node) = self.nodes.get(child) else {
            return Err(format!("appendChild child index {child} is out of range"));
        };
        if !matches!(
            &child_node.data,
            NodeData::Element(_) | NodeData::Text(_) | NodeData::DocumentFragment
        ) {
            return Err("appendChild child must be an Element, Text, or DocumentFragment".into());
        }

        let mut pending = vec![child];
        let mut visited = std::collections::HashSet::new();
        while let Some(descendant) = pending.pop() {
            if descendant == parent {
                return Err("appendChild would create a DOM cycle".into());
            }
            if visited.insert(descendant)
                && let Some(node) = self.nodes.get(descendant)
            {
                pending.extend(node.children.iter().copied());
            }
        }

        self.detach_from_parent(child);
        self.attach_child(parent, child);
        Ok(())
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
        self.set_element_namespace_info(id, ns, None);
    }

    /// Set an element namespace URI and its source prefix.
    pub fn set_element_namespace_info(
        &mut self,
        id: usize,
        ns: Option<SmolStr>,
        prefix: Option<SmolStr>,
    ) {
        let (namespace_changed, affects_tree_flags, affects_layout) = {
            let e = self.nodes[id]
                .data
                .as_element_mut()
                .expect("set_element_namespace called on non-Element");
            let changed = e.namespace != ns;
            let affects_tree_flags =
                changed && (e.tag_name.as_str() == "template" || e.tag_name.as_str() == "svg");
            let affects_layout = changed && e.tag_name.as_str() == "svg";
            e.namespace = ns;
            e.prefix = prefix;
            (changed, affects_tree_flags, affects_layout)
        };
        if namespace_changed && affects_tree_flags {
            self.flags_dirty = true;
        }
        if affects_layout {
            self.invalidate_layout_cache();
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
            .map(|(local, value)| Attr {
                namespace: None,
                prefix: None,
                local,
                value,
            })
            .collect();
    }

    /// Set one namespace-qualified attribute on an element.
    pub fn set_element_namespaced_attribute(
        &mut self,
        id: usize,
        namespace: impl Into<SmolStr>,
        prefix: Option<SmolStr>,
        local: impl Into<SmolStr>,
        value: impl Into<SmolStr>,
    ) -> Result<(), String> {
        let namespace = namespace.into();
        let local = local.into();
        if namespace.is_empty() || !is_valid_xml_name(local.as_str()) {
            return Err("invalid namespace-qualified attribute name".to_owned());
        }
        let value = value.into();
        let element = self.nodes[id]
            .data
            .as_element_mut()
            .expect("set_element_namespaced_attribute called on non-Element");
        if let Some(existing) = element.attributes.iter_mut().find(|attribute| {
            attribute.namespace.as_deref() == Some(namespace.as_str()) && attribute.local == local
        }) {
            existing.prefix = prefix;
            existing.value = value;
        } else {
            element.attributes.push(Attr {
                namespace: Some(namespace),
                prefix,
                local,
                value,
            });
        }
        Ok(())
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
            element
                .attributes
                .retain(|attr| attr.namespace.is_some() || attr.local != local);
            self.set_element_inline_style(id, Some(value));
            return Ok(());
        }

        let element = self.nodes[id]
            .data
            .as_element_mut()
            .expect("set_element_attribute called on non-Element");
        let mut found = false;
        element.attributes.retain_mut(|attr| {
            if attr.namespace.is_none() && attr.local == local {
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
            element.attributes.push(Attr {
                namespace: None,
                prefix: None,
                local,
                value,
            });
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
                .find(|attr| attr.namespace.is_none() && attr.local.as_str() == local)
                .map(|attr| attr.value.clone());
            element
                .attributes
                .retain(|attr| attr.namespace.is_some() || attr.local.as_str() != local);
            return Ok(element.inline_style.take().or(legacy_value));
        }

        let first_value = element
            .attributes
            .iter()
            .find(|attr| attr.namespace.is_none() && attr.local.as_str() == local)
            .map(|attr| attr.value.clone());
        element
            .attributes
            .retain(|attr| attr.namespace.is_some() || attr.local.as_str() != local);
        Ok(first_value)
    }

    /// Return an element's concatenated descendant text, excluding comments and processing instructions.
    pub fn element_text_content(&self, id: usize) -> Option<String> {
        let element = self.nodes.get(id)?;
        if !matches!(&element.data, NodeData::Element(_)) {
            return None;
        }
        let mut text = String::new();
        let mut pending: Vec<usize> = element.children.iter().rev().copied().collect();
        while let Some(child) = pending.pop() {
            let node = self.nodes.get(child)?;
            if let Some(content) = node.text_content() {
                text.push_str(content);
            } else {
                pending.extend(node.children.iter().rev().copied());
            }
        }
        Some(text)
    }

    /// Replace an element's children with a single text node, or no children for empty text.
    ///
    /// Removed nodes remain allocated in the arena but are detached, preserving stable handles.
    pub fn set_element_text_content(
        &mut self,
        id: usize,
        text: impl Into<SmolStr>,
    ) -> Result<(), String> {
        let Some(node) = self.nodes.get(id) else {
            return Err(format!("textContent target index {id} is out of range"));
        };
        if !matches!(&node.data, NodeData::Element(_)) {
            return Err(format!("textContent target index {id} is not an Element"));
        }
        let text = text.into();
        self.nodes[id].children.clear();
        if !text.is_empty() {
            let text_id = self.nodes.len();
            self.nodes.push(Node::new_text(text));
            self.nodes[id].children.push(text_id);
        }
        self.invalidate_layout_cache();
        self.flags_dirty = true;
        Ok(())
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
    /// Existing children are detached together by clearing their parent's
    /// child list. Tree changes mark layout caches and flat-tree membership
    /// dirty in the usual way. The source document is not modified.
    pub fn replace_children_from(
        &mut self,
        target_parent: usize,
        source_document: &Document,
        source_parent: usize,
    ) {
        let target_parent = self.nodes[target_parent]
            .template_contents()
            .unwrap_or(target_parent);
        if !self.nodes[target_parent].children.is_empty() {
            // The parent is already known. Avoid a whole-arena parent lookup
            // and shifting the remaining child IDs for every removed child.
            self.nodes[target_parent].children.clear();
            self.invalidate_layout_cache();
            self.flags_dirty = true;
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
                    self.set_element_namespace_info(
                        new_id,
                        element.namespace.clone(),
                        element.prefix.clone(),
                    );
                    self.set_element_attributes(
                        new_id,
                        element
                            .attributes
                            .iter()
                            .filter(|attr| {
                                attr.namespace.is_none() && attr.local.as_str() != "style"
                            })
                            .map(|attr| (attr.local.clone(), attr.value.clone()))
                            .collect(),
                    );
                    for attr in element
                        .attributes
                        .iter()
                        .filter(|attr| attr.namespace.is_some())
                    {
                        self.set_element_namespaced_attribute(
                            new_id,
                            attr.namespace.clone().unwrap_or_default(),
                            attr.prefix.clone(),
                            attr.local.clone(),
                            attr.value.clone(),
                        )
                        .expect("source namespace-qualified attribute was valid");
                    }

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
        const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
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
            node.set_inline_svg_content(false);
            node.set_inline_svg_root(false);
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
        let mut stack: Vec<(usize, bool, bool)> = vec![(root, false, false)];
        while let Some((id, in_template, in_svg_subtree)) = stack.pop() {
            let node = &mut self.nodes[id];
            let is_unrendered_by_kind = matches!(
                node.data,
                NodeData::Comment(_) | NodeData::ProcessingInstruction { .. }
            );
            node.set_in_document(!in_template && !is_unrendered_by_kind);
            let is_svg_element = matches!(
                &node.data,
                NodeData::Element(element)
                    if element.tag_name.as_str() == "svg"
                        && element.namespace.as_deref() == Some(SVG_NAMESPACE)
            );
            let svg_subtree_here = in_svg_subtree || is_svg_element;
            node.set_inline_svg_content(svg_subtree_here);
            node.set_inline_svg_root(is_svg_element && !in_svg_subtree);
            let is_template_here = match &node.data {
                NodeData::Element(element) => {
                    element.tag_name.as_str() == "template" && element.namespace.is_none()
                }
                _ => false,
            };
            let child_in_template = in_template || is_template_here;
            // Borrow the child IDs directly to avoid a temporary Vec per parent.
            stack.extend(
                node.children
                    .iter()
                    .rev()
                    .map(|&child| (child, child_in_template, svg_subtree_here)),
            );
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

    /// Serialize an inline SVG element and its subtree as a standalone XML
    /// source, retaining element/attribute namespace URIs and prefixes.
    pub fn serialize_svg_subtree(&self, id: usize) -> Result<Option<String>, String> {
        const SVG_NS: &str = "http://www.w3.org/2000/svg";
        const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
        const XMLNS_NS: &str = "http://www.w3.org/2000/xmlns/";

        enum Task {
            Node(usize),
            End(String),
        }

        let Some(root) = self.nodes.get(id) else {
            return Ok(None);
        };
        let NodeData::Element(root_element) = &root.data else {
            return Ok(None);
        };
        if root_element.tag_name.as_str() != "svg"
            || root_element.namespace.as_deref() != Some(SVG_NS)
        {
            return Ok(None);
        }

        let mut output = String::new();
        let mut pending = vec![Task::Node(id)];
        while let Some(task) = pending.pop() {
            match task {
                Task::End(name) => {
                    output.push_str("</");
                    output.push_str(&name);
                    output.push('>');
                }
                Task::Node(node_id) => {
                    let Some(node) = self.nodes.get(node_id) else {
                        return Err(format!("SVG subtree node {node_id} is out of range"));
                    };
                    match &node.data {
                        NodeData::Element(element) => {
                            let name = qualified_name(
                                element.prefix.as_deref(),
                                element.tag_name.as_str(),
                            );
                            if !is_valid_xml_name(element.tag_name.as_str())
                                || element
                                    .prefix
                                    .as_deref()
                                    .is_some_and(|prefix| !is_valid_xml_name(prefix))
                            {
                                return Err(format!("invalid SVG element name on node {node_id}"));
                            }
                            output.push('<');
                            output.push_str(&name);
                            let mut namespace_bindings = HashMap::new();
                            if let Some(namespace) = element.namespace.as_deref() {
                                let prefix = element.prefix.as_deref().unwrap_or("");
                                namespace_bindings.insert(prefix.to_owned(), namespace.to_owned());
                                let declaration = element.prefix.as_deref().map_or_else(
                                    || "xmlns".to_owned(),
                                    |prefix| format!("xmlns:{prefix}"),
                                );
                                output.push(' ');
                                output.push_str(&declaration);
                                output.push_str("=\"");
                                push_xml_escaped(&mut output, namespace, true);
                                output.push('"');
                            } else {
                                output.push_str(" xmlns=\"\"");
                            }

                            let mut generated_prefix = 0usize;
                            for attribute in &element.attributes {
                                if attribute.namespace.as_deref() == Some(XMLNS_NS)
                                    || (attribute.namespace.is_none()
                                        && attribute.local.as_str() == "style")
                                {
                                    continue;
                                }
                                if !is_valid_xml_name(attribute.local.as_str()) {
                                    return Err(format!(
                                        "invalid SVG attribute name {:?} on node {node_id}",
                                        attribute.local
                                    ));
                                }
                                let attribute_name = match attribute.namespace.as_deref() {
                                    None => attribute.local.to_string(),
                                    Some(XML_NS) => format!("xml:{}", attribute.local),
                                    Some(namespace) => {
                                        let source_prefix = attribute.prefix.as_deref();
                                        let prefix = match source_prefix {
                                            Some(prefix)
                                                if namespace_bindings
                                                    .get(prefix)
                                                    .is_none_or(|bound| bound == namespace) =>
                                            {
                                                prefix.to_owned()
                                            }
                                            _ => loop {
                                                generated_prefix += 1;
                                                let candidate =
                                                    format!("_raikiri_ns{generated_prefix}");
                                                if !namespace_bindings.contains_key(&candidate) {
                                                    break candidate;
                                                }
                                            },
                                        };
                                        if !is_valid_xml_name(&prefix) {
                                            return Err(format!(
                                                "invalid SVG attribute prefix {prefix:?} on node {node_id}"
                                            ));
                                        }
                                        if !namespace_bindings.contains_key(&prefix) {
                                            namespace_bindings
                                                .insert(prefix.clone(), namespace.to_owned());
                                            let declaration = format!("xmlns:{prefix}");
                                            output.push(' ');
                                            output.push_str(&declaration);
                                            output.push_str("=\"");
                                            push_xml_escaped(&mut output, namespace, true);
                                            output.push('"');
                                        }
                                        format!("{prefix}:{}", attribute.local)
                                    }
                                };
                                output.push(' ');
                                output.push_str(&attribute_name);
                                output.push_str("=\"");
                                push_xml_escaped(&mut output, attribute.value.as_str(), true);
                                output.push('"');
                            }
                            if let Some(style) = &element.inline_style {
                                output.push_str(" style=\"");
                                push_xml_escaped(&mut output, style.as_str(), true);
                                output.push('"');
                            }
                            output.push('>');
                            pending.push(Task::End(name));
                            pending.extend(node.children.iter().rev().copied().map(Task::Node));
                        }
                        NodeData::Text(text) => {
                            push_xml_escaped(&mut output, text.text_content.as_str(), false);
                        }
                        NodeData::Document | NodeData::Comment(_) => {}
                        NodeData::ProcessingInstruction { .. } | NodeData::DocumentFragment => {}
                    }
                }
            }
        }
        Ok(Some(output))
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
    pub(crate) fn invalidate_layout_cache(&mut self) {
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
mod tests;
