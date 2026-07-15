//! RaikiriTreeSink — html5ever `TreeSink` implementation backed by
//! raikiri-dom::Document. Handle = arena index (usize).

use std::borrow::Cow;
use std::cell::{Cell, Ref, RefCell};

use html5ever::interface::{
    Attribute, ElementFlags, NodeOrText, QualName, TreeSink,
};
use html5ever::tendril::StrTendril;
use html5ever::tree_builder::QuirksMode;
use markup5ever::ns;
use raikiri_dom::Document;
use raikiri_traits::{Dom, RenderWarning, WarningKind};
use rustc_hash::{FxHashMap, FxHashSet};
use smol_str::SmolStr;
use taffy::Style;

use crate::types::UncascadedDocument;

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
        let idx = self
            .document
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

        let stylesheet_sources = extract_inline_stylesheets(&document);
        strip_non_element_stubs(&mut document);
        UncascadedDocument {
            dom: document,
            stylesheet_sources,
            warnings,
            quirks_mode: convert_quirks(self.quirks_mode.get()),
        }
    }

    fn parse_error(&self, msg: Cow<'static, str>) {
        self.warnings.borrow_mut().push(RenderWarning {
            kind: WarningKind::HtmlParseError {
                message: msg.into_owned(),
            },
            node_id: None,
            details: String::new(),
        });
    }

    fn get_document(&self) -> usize {
        self.document.borrow().root_id().0 as usize
    }

    fn elem_name<'a>(&'a self, target: &'a usize) -> Ref<'a, QualName> {
        Ref::map(self.qual_names.borrow(), |m| {
            m.get(target).expect("elem_name called on non-element handle")
        })
    }

    fn create_element(
        &self,
        name: QualName,
        attrs: Vec<Attribute>,
        _flags: ElementFlags,
    ) -> usize {
        self.make_element(name, attrs)
    }

    fn create_comment(&self, text: StrTendril) -> usize {
        // M1 workaround: comment node を "#comment" tag の element として保持。
        // Task 9 で NodeKind::Comment を検討 (m1.5 の dom-model 昇格候補)。
        let idx = self
            .document
            .borrow_mut()
            .append_element(None, "#comment", Style::default(), None::<&str>);
        // side-table には登録しない (elem_name 呼ばれるべきではない)
        let _ = text;
        idx
    }

    fn create_pi(&self, target: StrTendril, data: StrTendril) -> usize {
        let idx = self
            .document
            .borrow_mut()
            .append_element(None, "#pi", Style::default(), None::<&str>);
        let _ = (target, data);
        idx
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

    fn append_doctype_to_document(
        &self,
        _name: StrTendril,
        _public_id: StrTendril,
        _system_id: StrTendril,
    ) {
        // M1 では doctype node 化しない。quirks_mode は別 callback で通知される。
    }

    fn get_template_contents(&self, target: &usize) -> usize {
        // M1 では template contents = template element 自身 (真の template
        // fragment 分離は M1 spike scope 外)。返り値の Handle が children
        // 取得に使われる想定の callers に対する minimum viable。
        *target
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
        self.document.borrow_mut().reparent_children(*node, *new_parent);
    }

    fn is_mathml_annotation_xml_integration_point(&self, _handle: &usize) -> bool {
        false
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
        if let Some(node) = doc.node(id)
            && let Some(el) = node.as_element()
        {
            match el.tag_name() {
                "style" => {
                    // 直下の Text children を concat
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
                "template" => {
                    // spec 上 inert (bd-xno)
                    continue;
                }
                _ => {}
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
                // (dom_impl 側で filter される)。
                inline_style = Some(SmolStr::new(a.value.as_ref()));
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

/// html5ever が emit した comment / processing-instruction を parent から detach する。
/// M1 spike では raikiri-dom::Node は Element / Text / Document のみ表現できるため、
/// `create_comment` / `create_pi` は `#comment` / `#pi` tag の element として保持され
/// ている。これらを DOM tree から除去することで cascade / selector matching が誤って
/// 拾わないようにする。arena からは削除しない (index の再利用が起こらないため無害)。
/// NodeKind への Comment / ProcessingInstruction 追加 (恒久対応) は別 issue に defer。
///
/// Single-pass O(N + total_children) 実装: 全 node をスキャンし、children 内に
/// stub tag を含む node について children Vec を一度だけ retain。per-stub の
/// parent_of + children.remove(pos) を回避 (attacker-controlled な多量 comment で
/// quadratic を防ぐ)。
fn strip_non_element_stubs(doc: &mut Document) {
    use raikiri_traits::{Dom, Element, Node};

    // Pass 1: 全 node をスキャンし、tag が "#comment" / "#pi" の arena index を集める。
    let mut stub_indices = FxHashSet::default();
    let root = doc.root_id();
    let mut stack: Vec<raikiri_traits::NodeId> = vec![root];
    while let Some(id) = stack.pop() {
        if let Some(node) = doc.node(id)
            && let Some(el) = node.as_element()
            && matches!(el.tag_name(), "#comment" | "#pi")
        {
            stub_indices.insert(id.0 as usize);
        }
        let kids: Vec<_> = doc.child_ids(id).collect();
        for c in kids.into_iter().rev() {
            stack.push(c);
        }
    }

    if stub_indices.is_empty() {
        return;
    }

    // Pass 2: 各 parent の children を single retain で filter。
    doc.retain_children(|c| !stub_indices.contains(&c));
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
