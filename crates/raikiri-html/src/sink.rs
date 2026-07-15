//! RaikiriTreeSink — html5ever `TreeSink` implementation backed by
//! raikiri-dom::Document. Handle = arena index (usize).

use std::borrow::Cow;
use std::cell::{Cell, Ref, RefCell};

use html5ever::interface::{
    Attribute, ElementFlags, NodeOrText, QualName, TreeSink,
};
use html5ever::tendril::StrTendril;
use html5ever::tree_builder::QuirksMode;
use raikiri_dom::Document;
use raikiri_traits::{Dom, Element, Node, RenderWarning, WarningKind};
use rustc_hash::FxHashMap;
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
    /// 返り値 `Ref<'_, QualName>` の裏にある。
    qual_names: RefCell<FxHashMap<usize, QualName>>,
    /// Handle → attribute 列。M1.3 では merge (add_attrs_if_missing) のみ運用。
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
    fn make_element(&self, name: QualName, attrs: Vec<Attribute>) -> usize {
        let tag: SmolStr = name.local.as_ref().into();
        let idx = self
            .document
            .borrow_mut()
            .append_element(None, tag, Style::default());
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
        let document = self.document.into_inner();
        let warnings = self.warnings.into_inner();
        let stylesheet_sources = extract_inline_stylesheets(&document);
        UncascadedDocument {
            dom: document,
            stylesheet_sources,
            warnings,
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
            .append_element(None, "#comment", Style::default());
        // side-table には登録しない (elem_name 呼ばれるべきではない)
        let _ = text;
        idx
    }

    fn create_pi(&self, target: StrTendril, data: StrTendril) -> usize {
        let idx = self
            .document
            .borrow_mut()
            .append_element(None, "#pi", Style::default());
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

/// `<style>` element の text content を DFS で集約する。
/// Text node の text_content を concat して 1 stylesheet 相当として push。
fn extract_inline_stylesheets(doc: &Document) -> Vec<String> {
    fn walk(
        doc: &Document,
        id: raikiri_traits::NodeId,
        out: &mut Vec<String>,
    ) {
        if let Some(node) = doc.node(id)
            && let Some(el) = node.as_element()
            && el.tag_name() == "style"
        {
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
            // <style> の内容は CSS のみ想定、再帰入る必要なし
            return;
        }
        for c in doc.child_ids(id) {
            walk(doc, c, out);
        }
    }
    let mut out = Vec::new();
    walk(doc, doc.root_id(), &mut out);
    out
}
