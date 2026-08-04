//! RaikiriTreeSink — html5ever `TreeSink` implementation backed by
//! raikiri-dom::Document. Handle = arena index (usize).

use std::borrow::Cow;
use std::cell::{Cell, Ref, RefCell};

use html5ever::interface::{Attribute, ElementFlags, NodeOrText, QualName, TreeSink};
use html5ever::tendril::StrTendril;
use html5ever::tree_builder::QuirksMode;
use markup5ever::ns;
use raikiri_dom::Document;
use raikiri_traits::{Dom, RenderWarning, WarningKind};
use rustc_hash::FxHashMap;
use smol_str::SmolStr;
use taffy::Style;

use crate::types::UncascadedDocument;

/// `parse_error` が保持する [`RenderWarning`] 件数の上限 (Codex Security
/// finding raikiri-spike-g9vr、SEC HIGH)。
///
/// html5ever は malformed input の小さな token 1 個あたり概ね 1 個の
/// parse_error を報告しうるため、cap が無いと attacker が任意個数の owned
/// `String` 持ち `RenderWarning` を積ませて memory を枯渇させられる (PoC:
/// 100,000 個の `</x>` で 100,001 warnings、RSS 線形増加を実測)。
/// `raikiri_traits::RenderLimits::max_input_bytes` (bd raikiri-spike-4kw) は
/// 別軸の入力 byte 数 cap であり、この per-token 増幅は防がない。
///
/// Option B stopgap (bd raikiri-spike-d9y.3 の parse_html DoS fix と同じ
/// pattern): `raikiri-html` crate 内で完結する local const。`RenderLimits`
/// へ昇格する Option A は follow-up task に defer (raikiri-spike-4kw 参照)。
pub(crate) const MAX_HTML_PARSE_WARNINGS: usize = 1024;

/// html5ever `TreeSink` の raikiri 実装。Handle は raikiri-dom arena の
/// index (`usize`)、Output は [`UncascadedDocument`]。QualName / Attribute
/// の side-table を RefCell 内 FxHashMap で保持し、raikiri-dom::Node に
/// html5ever 固有型を漏らさない (cleanroom)。
pub struct RaikiriTreeSink {
    document: RefCell<Document>,
    /// Handle → 完全な QualName (namespace + local)。`elem_name()` の
    /// 返り値 `Ref<'_, QualName>` の裏にある。`finish()` 時に non-HTML namespace
    /// のみ raikiri-dom::Node.namespace に wire (raikiri-spike-blg)。
    qual_names: RefCell<FxHashMap<usize, QualName>>,
    /// Handle → attribute 列。parse 中は merge (add_attrs_if_missing) で更新。
    /// `finish()` 時に null-namespace attr を raikiri-dom::Node.attributes に、
    /// `style` attribute のみ raikiri-dom::Node.inline_style に分離して wire
    /// (raikiri-spike-blg)。namespaced attr (xlink:href 等) は M2+ に defer。
    attributes: RefCell<FxHashMap<usize, Vec<Attribute>>>,
    /// html5ever が報告した非致命 parse error の buffer。finish() で
    /// UncascadedDocument.warnings に移設。
    warnings: RefCell<Vec<RenderWarning>>,
    /// Document 全体の quirks mode。cascade phase (m1.4) が参照する予定。
    quirks_mode: Cell<QuirksMode>,
}

impl RaikiriTreeSink {
    /// 新規 sink を construct。Document は arena index 0 に virtual root を持つ
    /// 空 Document で初期化される。
    pub fn new() -> Self {
        Self {
            document: RefCell::new(Document::new()),
            qual_names: RefCell::new(FxHashMap::default()),
            attributes: RefCell::new(FxHashMap::default()),
            warnings: RefCell::new(Vec::new()),
            quirks_mode: Cell::new(QuirksMode::NoQuirks),
        }
    }

    /// Element 用の detached node を construct し、side-table に QualName /
    /// attrs を登録する。
    ///
    /// `Node.inline_style` / `Node.namespace` / `Node.attributes` の wiring は
    /// [`RaikiriTreeSink::finish`] で side-table から一括 populate する
    /// (raikiri-spike-blg)。ここでは Document への tag + default Style 登録と
    /// side-table への full-fidelity 保存のみ行う。
    fn make_element(&self, name: QualName, attrs: Vec<Attribute>) -> usize {
        let tag: SmolStr = name.local.as_ref().into();
        let idx =
            self.document
                .borrow_mut()
                .append_element(None, tag, Style::default(), None::<&str>);
        self.qual_names.borrow_mut().insert(idx, name);
        self.attributes.borrow_mut().insert(idx, attrs);
        idx
    }

    /// Text を parent の最後の child に append。
    ///
    /// Note: TreeSink 契約 "既に末尾 child が Text なら concat" は M1 spike
    /// では実装せず、毎回新規 Text node を作る (raikiri-dom::Node.text_content
    /// が SmolStr で pit-of-success な mutate API 未整備のため)。parley layout
    /// は adjacent Text を単一 inline run として処理する想定。
    fn append_text_smart(&self, parent: usize, text: StrTendril) {
        self.document
            .borrow_mut()
            .append_text(parent, text.to_string());
    }
}

impl Default for RaikiriTreeSink {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeSink for RaikiriTreeSink {
    type Handle = usize;
    type Output = UncascadedDocument;
    type ElemName<'a> = Ref<'a, QualName>;

    fn finish(self) -> UncascadedDocument {
        let mut document = self.document.into_inner();
        let warnings = self.warnings.into_inner();
        let qual_names = self.qual_names.into_inner();
        let attributes = self.attributes.into_inner();

        // raikiri-spike-blg: side-table を raikiri-dom::Node に wire。
        // qual_names → Node.namespace (non-HTML のみ)。
        // attributes → Node.attributes (null-ns、style を除く) + Node.inline_style。
        wire_side_tables(&mut document, &qual_names, &attributes);

        // raikiri-spike-84y: 旧 strip_non_element_stubs (pseudo-tag な
        // "#comment" / "#pi" Element を tree から physical 除去) を廃止。
        // Comment / ProcessingInstruction は NodeData::Comment /
        // NodeData::ProcessingInstruction variant として tree 内に persist する
        // (WHATWG DOM §4 NodeType との alignment)。両 variant は
        // `mark_in_document_flags` step 2 で IS_IN_DOCUMENT bit が clear される
        // 契約 (advisor step-6 option (i))、したがって:
        // - TaffyChildIter の is_in_document filter で layout child count に leak
        //   しない (raikiri-dom/src/taffy_impl.rs:38,61,71 の filter)
        // - extract_inline_stylesheets / find_head_element / find_body の
        //   is_in_document() gate + Element gate で自動 skip
        // - cascade / paint 全 traversal も同 gate で skip
        //
        // raikiri-spike-37c: template subtree の IS_IN_DOCUMENT bit を clear
        // + detached node (foster parenting transient) + Comment/PI (84y) の
        // bit を clear する。extract_inline_stylesheets が is_in_document() gate
        // 経由でこれらを skip するため、その前に走らせる。
        document.mark_in_document_flags();

        let stylesheet_sources = extract_inline_stylesheets(&document);
        UncascadedDocument {
            dom: document,
            stylesheet_sources,
            warnings,
            quirks_mode: convert_quirks(self.quirks_mode.get()),
        }
    }

    fn parse_error(&self, msg: Cow<'static, str>) {
        // 最後の 1 slot は「以降 suppress した」ことを示す synthetic entry
        // 専用に予約する (silent drop だと 1024 件と 8,000,000 件を consumer
        // が区別できない)。よって実際の parse error は MAX-1 件まで記録する。
        const LAST_REAL_SLOT: usize = MAX_HTML_PARSE_WARNINGS - 1;

        let mut warnings = self.warnings.borrow_mut();
        match warnings.len().cmp(&LAST_REAL_SLOT) {
            std::cmp::Ordering::Less => {
                warnings.push(RenderWarning {
                    kind: WarningKind::HtmlParseError {
                        message: msg.into_owned(),
                    },
                    node_id: None,
                    details: String::new(),
                });
            }
            std::cmp::Ordering::Equal => {
                warnings.push(RenderWarning {
                    kind: WarningKind::HtmlParseError {
                        message: format!(
                            "further parse errors suppressed after reaching the {MAX_HTML_PARSE_WARNINGS}-warning cap"
                        ),
                    },
                    node_id: None,
                    details: String::new(),
                });
            }
            std::cmp::Ordering::Greater => {}
        }
    }

    fn get_document(&self) -> usize {
        self.document.borrow().root_id().0 as usize
    }

    fn elem_name<'a>(&'a self, target: &'a usize) -> Ref<'a, QualName> {
        Ref::map(self.qual_names.borrow(), |m| {
            m.get(target)
                .expect("elem_name called on non-element handle")
        })
    }

    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> usize {
        let idx = self.make_element(name, attrs);
        // raikiri-spike-xno Part 2: html5ever は `<template>` element を作る時
        // `flags.template=true` を渡す (markup5ever `create_element_with_flags`)。
        // その場で fragment root を eager allocate + template_contents slot に
        // wire することで、後続の `TreeSink::append(get_template_contents(t), ...)`
        // が fragment root に子を積む。template element 自身の children は
        // 空のまま (blitz と同じ shape、詳細は
        // `Document::allocate_template_fragment_root` doc)。
        if flags.template {
            self.document
                .borrow_mut()
                .allocate_template_fragment_root(idx);
        }
        idx
    }

    fn create_comment(&self, text: StrTendril) -> usize {
        // raikiri-spike-84y: NodeData::Comment variant として恒久保持
        // (旧 M1: "#comment" pseudo-tag Element + sink.finish 内で strip)。
        // detached (parent=None) で allocate、html5ever が後で append(parent, ...)
        // で attach する。
        self.document
            .borrow_mut()
            .append_comment(None, text.to_string())
    }

    fn create_pi(&self, target: StrTendril, data: StrTendril) -> usize {
        // raikiri-spike-84y: NodeData::ProcessingInstruction variant として
        // 恒久保持 (旧 M1: "#pi" pseudo-tag Element + strip)。
        self.document.borrow_mut().append_processing_instruction(
            None,
            target.to_string(),
            data.to_string(),
        )
    }

    fn append(&self, parent: &usize, child: NodeOrText<usize>) {
        match child {
            NodeOrText::AppendNode(c) => {
                self.document.borrow_mut().attach_child(*parent, c);
            }
            NodeOrText::AppendText(text) => {
                self.append_text_smart(*parent, text);
            }
        }
    }

    fn append_based_on_parent_node(
        &self,
        element: &usize,
        prev_element: &usize,
        child: NodeOrText<usize>,
    ) {
        // Task 9 で foster parenting の proper impl を書く。M1 hello-world
        // では発火しないので defensive fallback: parent がいれば insert_before、
        // いなければ prev_element の子に append。
        let has_parent = self.document.borrow().parent_of(*element).is_some();
        if has_parent {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev_element, child);
        }
    }

    // NB (raikiri-spike-84y): raikiri-dom は NodeKind::Comment /
    // NodeKind::ProcessingInstruction を variant として持つようになったため
    // (旧 Task 9 pending item)、create_comment / create_pi は上で直接
    // NodeData variant を allocate している。旧 `#comment` / `#pi` pseudo-tag
    // + strip_non_element_stubs 二段構えは廃止。

    fn append_doctype_to_document(
        &self,
        _name: StrTendril,
        _public_id: StrTendril,
        _system_id: StrTendril,
    ) {
        // M1 では doctype node 化しない。quirks_mode は別 callback で通知される。
    }

    fn get_template_contents(&self, target: &usize) -> usize {
        // raikiri-spike-xno Part 2: `create_element` が `flags.template=true`
        // を観測した時 `template_contents` slot に fragment root の arena index
        // を wire している。ここで返した index が html5ever の以降の
        // `TreeSink::append` の parent handle として使われる (template contents
        // は fragment root の子として積まれ、template element 自身は空の
        // children を保つ)。
        //
        // Defensive fallback: sink 経由でない直接組み立て (unit test 等) で
        // `template_contents` が未 wire な場合は旧挙動どおり `*target` を返す。
        // Node::is_in_document() gate は 37c 経路 (直接構築時の
        // `mark_in_document_flags` の template skip) が引き続き cover するので
        // silent bug には至らない。
        let doc = self.document.borrow();
        doc.get_node(*target)
            .and_then(|n| n.template_contents())
            .unwrap_or(*target)
    }

    fn same_node(&self, x: &usize, y: &usize) -> bool {
        x == y
    }

    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.quirks_mode.set(mode);
    }

    fn append_before_sibling(&self, sibling: &usize, new_node: NodeOrText<usize>) {
        let parent = self
            .document
            .borrow()
            .parent_of(*sibling)
            .expect("append_before_sibling: sibling has no parent");
        match new_node {
            NodeOrText::AppendNode(c) => {
                // detach if already attached (TreeSink 契約: new_node は old
                // parent を持ちうる)
                self.document.borrow_mut().detach_from_parent(c);
                self.document
                    .borrow_mut()
                    .insert_child_before(parent, *sibling, c);
            }
            NodeOrText::AppendText(text) => {
                // 新規 Text node を arena に作成 (detached にできない — append_text
                // が parent 必須のため、まず parent 末尾に append → 直後 detach
                // → insert_before の 3 step)。
                let text_id = self
                    .document
                    .borrow_mut()
                    .append_text(parent, text.to_string());
                self.document.borrow_mut().detach_from_parent(text_id);
                self.document
                    .borrow_mut()
                    .insert_child_before(parent, *sibling, text_id);
            }
        }
    }

    fn add_attrs_if_missing(&self, target: &usize, attrs: Vec<Attribute>) {
        let mut store = self.attributes.borrow_mut();
        let existing = store.entry(*target).or_default();
        for a in attrs {
            let name_exists = existing.iter().any(|e| e.name == a.name);
            if !name_exists {
                existing.push(a);
            }
        }
    }

    fn remove_from_parent(&self, target: &usize) {
        self.document.borrow_mut().detach_from_parent(*target);
    }

    fn reparent_children(&self, node: &usize, new_parent: &usize) {
        self.document
            .borrow_mut()
            .reparent_children(*node, *new_parent);
    }

    fn is_mathml_annotation_xml_integration_point(&self, handle: &usize) -> bool {
        // HTML5 §13.2.5 Tree construction: MathML `annotation-xml` element is
        // an HTML integration point iff its `encoding` attribute value is an
        // ASCII case-insensitive match for `text/html` or `application/xhtml+xml`.
        // side-table (qual_names + attributes) から直接判定する。
        let qual_names = self.qual_names.borrow();
        let Some(name) = qual_names.get(handle) else {
            return false;
        };
        if name.ns != ns!(mathml) || name.local.as_ref() != "annotation-xml" {
            return false;
        }
        let attributes = self.attributes.borrow();
        let Some(attrs) = attributes.get(handle) else {
            return false;
        };
        // HTML spec §13.2.5.32: duplicate attribute → ignore later occurrences
        // (first-wins)。`wire_side_tables` / `sink_first_wins_on_duplicate_style_attribute`
        // で pin されている契約と整合させるため、any() ではなく find() で最初の
        // null-ns encoding attr を取り、その value のみで判定する。
        let Some(encoding) = attrs
            .iter()
            .find(|a| a.name.ns == ns!() && a.name.local.as_ref() == "encoding")
        else {
            return false;
        };
        encoding.value.eq_ignore_ascii_case("text/html")
            || encoding.value.eq_ignore_ascii_case("application/xhtml+xml")
    }
}

/// `<head>` 内の `<style>` element の text content を DFS (iterative) で
/// document order 集約する。
///
/// 設計仕様書 §6 MVP: `<head>` 内の `<style>` のみ head 内出現順で登録。
/// `<body>` 内の `<style>` は "出現位置以降のみ有効" という position-aware
/// semantics が必要なため後の拡張として defer。
///
/// `<template>` subtree は spec 上 inert なので skip (raikiri-spike-xno 参照)。
/// 明示的 stack を使うことで attacker-controlled な深い DOM でも stack
/// overflow を起こさない。
fn extract_inline_stylesheets(doc: &Document) -> Vec<String> {
    use raikiri_traits::{Dom, Element, Node};

    // まず <head> element を探す。存在しなければ MVP scope 上 stylesheet なし。
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
            if let Some(el) = node.as_element()
                && el.tag_name() == "style"
            {
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
        // Push in reverse so LIFO pop yields document order (source-order for
        // CSS cascade tie-breaking, deterministic for detach batching).
        let kids: Vec<_> = doc.child_ids(id).collect();
        for c in kids.into_iter().rev() {
            stack.push(c);
        }
    }
    out
}

/// Document tree の `<head>` element を DFS (iterative) で探す。
/// 通常 `<html>` の直下 first-child だが html5ever tree building で位置が
/// 変わる場合もあるので linear scan。見つからない場合 None。
fn find_head_element(doc: &Document) -> Option<raikiri_traits::NodeId> {
    use raikiri_traits::{Dom, Element, Node};

    let mut stack: Vec<raikiri_traits::NodeId> = vec![doc.root_id()];
    while let Some(id) = stack.pop() {
        if let Some(node) = doc.node(id)
            && let Some(el) = node.as_element()
            && el.tag_name() == "head"
        {
            return Some(id);
        }
        let kids: Vec<_> = doc.child_ids(id).collect();
        for c in kids.into_iter().rev() {
            stack.push(c);
        }
    }
    None
}

/// side-table (`qual_names` / `attributes`) の内容を raikiri-dom::Node に写す。
///
/// - `qual_names`: element の namespace URI が HTML default (`ns!(html)`) 以外
///   なら `Node.namespace` に格納 (HTML default は `None` fast path のまま)。
/// - `attributes`: null-namespace attr のみ raikiri-dom に運ぶ (namespaced attr
///   = `xlink:href` on SVG 等は M2+ に defer)。`style` attr は `Node.inline_style`
///   に分離、それ以外は `Node.attributes` の順序保持 Vec に格納。
///
/// `finish()` 時に一度だけ呼ばれる single-pass 変換。parse 中は side-table
/// (RefCell) のみ更新し Node は無変更、finish で bulk populate することで
/// html5ever が add_attrs_if_missing / create_element の順序で attr を差し込む
/// 呼び出しパターンを気にせず済む。
fn wire_side_tables(
    doc: &mut Document,
    qual_names: &FxHashMap<usize, QualName>,
    attributes: &FxHashMap<usize, Vec<Attribute>>,
) {
    for (idx, name) in qual_names {
        // HTML default namespace は Node.namespace = None のまま (fast path)。
        // それ以外の svg / mathml / xml / ... は URI string を SmolStr で格納。
        if name.ns != ns!(html) {
            doc.set_element_namespace(*idx, Some(SmolStr::new(name.ns.as_ref())));
        }
    }
    for (idx, attrs) in attributes {
        let mut inline_style: Option<SmolStr> = None;
        let mut native: Vec<(SmolStr, SmolStr)> = Vec::with_capacity(attrs.len());
        for a in attrs {
            // null namespace 以外の attr (xlink:href 等) は M1 では drop。SVG /
            // MathML full support は M2+ の別 issue で扱う。
            if a.name.ns != ns!() {
                continue;
            }
            let local = a.name.local.as_ref();
            if local == "style" {
                // 空文字列 `style=""` は Element trait contract 上 None なので
                // ここでは boundary 正規化せず raw 値のまま Node に格納
                // (dom_impl 側で filter される)。html5ever は attrs を parse
                // 段で dedupe する想定だが、契約に依存せず defensive に
                // first-wins (Node.attributes の find() first-match 挙動と
                // 整合、HTML spec §13.2.5.32 の "duplicate-attribute → ignore
                // later occurrences" とも整合)。
                if inline_style.is_none() {
                    inline_style = Some(SmolStr::new(a.value.as_ref()));
                }
                continue;
            }
            native.push((SmolStr::new(local), SmolStr::new(a.value.as_ref())));
        }
        if let Some(source) = inline_style {
            doc.set_element_inline_style(*idx, Some(source));
        }
        if !native.is_empty() {
            doc.set_element_attributes(*idx, native);
        }
    }
}

/// html5ever `QuirksMode` を raikiri-native `QuirksMode` へ変換する
/// (cleanroom boundary: html5ever 型を raikiri-traits に持ち込まない)。
fn convert_quirks(mode: QuirksMode) -> raikiri_traits::QuirksMode {
    match mode {
        QuirksMode::Quirks => raikiri_traits::QuirksMode::Quirks,
        QuirksMode::LimitedQuirks => raikiri_traits::QuirksMode::LimitedQuirks,
        QuirksMode::NoQuirks => raikiri_traits::QuirksMode::NoQuirks,
    }
}
