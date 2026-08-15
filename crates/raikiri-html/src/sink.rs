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

/// html5ever `TreeSink` の raikiri 実装。Handle は raikiri-dom arena の
/// index (`usize`)、Output は [`UncascadedDocument`]。QualName / Attribute
/// の side-table を RefCell 内 FxHashMap で保持し、raikiri-dom::Node に
/// html5ever 固有型を漏らさない (cleanroom)。
pub struct RaikiriTreeSink {
    document: RefCell<Document>,
    /// Handle → 完全な QualName (namespace + local)。`elem_name()` の
    /// 返り値 `Ref<'_, QualName>` の裏にある。`finish()` 時に non-HTML namespace
    /// のみ raikiri-dom::Node.namespace に wire する。
    qual_names: RefCell<FxHashMap<usize, QualName>>,
    /// Handle → attribute 列。parse 中は merge (add_attrs_if_missing) で更新。
    /// `finish()` 時に null-namespace attr を raikiri-dom::Node.attributes に、
    /// `style` attribute のみ raikiri-dom::Node.inline_style に分離して wire
    /// namespaced attr (xlink:href 等) は将来に defer。
    attributes: RefCell<FxHashMap<usize, Vec<Attribute>>>,
    /// html5ever が報告した非致命 parse error の buffer。finish() で
    /// UncascadedDocument.warnings に移設。
    warnings: RefCell<Vec<RenderWarning>>,
    /// Document 全体の quirks mode。cascade phase が参照する予定。
    quirks_mode: Cell<QuirksMode>,
    /// `parse_error` が `warnings` に記録する件数の上限 (`None` = 無制限)。
    /// [`raikiri_traits::RenderLimits::max_parse_warnings`] から consult
    /// される想定の値 — 詳しい rationale はそちらの field doc を参照。
    max_parse_warnings: Option<usize>,
}

impl RaikiriTreeSink {
    /// 新規 sink を construct。Document は arena index 0 に virtual root を持つ
    /// 空 Document で初期化される。
    ///
    /// `max_parse_warnings` は [`TreeSink::parse_error`] が記録する warning
    /// 件数の上限 (`None` = 無制限)。呼び出し側は通常
    /// `raikiri_traits::RenderLimits::max_parse_warnings` の値をそのまま渡す
    /// (`raikiri_html::parse` は default 値、`raikiri::parse_html_with_limits`
    /// は consumer が設定した値を渡す)。
    pub fn new(max_parse_warnings: Option<usize>) -> Self {
        Self {
            document: RefCell::new(Document::new()),
            qual_names: RefCell::new(FxHashMap::default()),
            attributes: RefCell::new(FxHashMap::default()),
            warnings: RefCell::new(Vec::new()),
            quirks_mode: Cell::new(QuirksMode::NoQuirks),
            max_parse_warnings,
        }
    }

    /// Element 用の detached node を construct し、side-table に QualName /
    /// attrs を登録する。
    ///
    /// `Node.inline_style` / `Node.namespace` / `Node.attributes` の wiring は
    /// [`RaikiriTreeSink::finish`] で side-table から一括 populate する
    /// ここでは Document への tag + default Style 登録と
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
    /// Note: TreeSink 契約 "既に末尾 child が Text なら concat" は現状の spike
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
        Self::new(raikiri_traits::RenderLimits::default().max_parse_warnings)
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

        // side-table を raikiri-dom::Node に wire。
        // qual_names → Node.namespace (non-HTML のみ)。
        // attributes → Node.attributes (null-ns、style を除く) + Node.inline_style。
        wire_side_tables(&mut document, &qual_names, &attributes);

        // 旧 strip_non_element_stubs (pseudo-tag な
        // "#comment" / "#pi" Element を tree から physical 除去) を廃止。
        // Comment / ProcessingInstruction は NodeData::Comment /
        // NodeData::ProcessingInstruction variant として tree 内に persist する
        // (WHATWG DOM §4 NodeType との alignment)。両 variant は
        // `mark_in_document_flags` step 2 で IS_IN_DOCUMENT bit が clear される
        // 契約であり、したがって:
        // - TaffyChildIter の is_in_document filter で layout child count に leak
        //   しない (raikiri-dom/src/taffy_impl.rs:38,61,71 の filter)
        // - extract_inline_stylesheets / find_head_element / find_body の
        //   is_in_document() gate + Element gate で自動 skip
        // - cascade / paint 全 traversal も同 gate で skip
        //
        // template subtree の IS_IN_DOCUMENT bit を clear
        // + detached node (foster parenting transient) + Comment/PI の
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
        let mut warnings = self.warnings.borrow_mut();

        let Some(cap) = self.max_parse_warnings else {
            // 無制限 (Consumer が明示的に `max_parse_warnings = None` を
            // 設定した場合のみ到達する)。
            warnings.push(RenderWarning {
                kind: WarningKind::HtmlParseError {
                    message: msg.into_owned(),
                },
                node_id: None,
                details: String::new(),
            });
            return;
        };
        if cap == 0 {
            // 実 warning 用の slot は無いが、cap>0 の場合と同じく「1件以上の
            // parse error が発生したが suppress された」ことを示す synthetic
            // entry を最初の呼び出し時にだけ 1 件積む。無条件 no-op だと
            // parse error が 0 件だったケースと区別できず、cap>0 の
            // trip-and-record semantics と非対称な silent disable になる。
            if warnings.is_empty() {
                warnings.push(RenderWarning {
                    kind: WarningKind::HtmlParseError {
                        message: "parse errors suppressed (max_parse_warnings = 0)".to_string(),
                    },
                    node_id: None,
                    details: String::new(),
                });
            }
            return;
        }

        // 最後の 1 slot は「以降 suppress した」ことを示す synthetic entry
        // 専用に予約する (silent drop だと cap 件とそれを大幅に超える件数を
        // consumer が区別できない)。よって実際の parse error は cap-1 件まで
        // 記録する。
        let last_real_slot = cap - 1;
        match warnings.len().cmp(&last_real_slot) {
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
                            "further parse errors suppressed after reaching the {cap}-warning cap"
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
        // html5ever は `<template>` element を作る時
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
        // NodeData::Comment variant として恒久保持
        // (以前は "#comment" pseudo-tag Element + sink.finish 内で strip)。
        // detached (parent=None) で allocate、html5ever が後で append(parent, ...)
        // で attach する。
        self.document
            .borrow_mut()
            .append_comment(None, text.to_string())
    }

    fn create_pi(&self, target: StrTendril, data: StrTendril) -> usize {
        // NodeData::ProcessingInstruction variant として
        // 恒久保持 (以前は "#pi" pseudo-tag Element + strip)。
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
        // 将来 foster parenting の proper impl を書く。現状の hello-world
        // では発火しないので defensive fallback: parent がいれば insert_before、
        // いなければ prev_element の子に append。
        let has_parent = self.document.borrow().parent_of(*element).is_some();
        if has_parent {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev_element, child);
        }
    }

    // NB: raikiri-dom は NodeKind::Comment /
    // NodeKind::ProcessingInstruction を variant として持つようになったため、
    // create_comment / create_pi は上で直接
    // NodeData variant を allocate している。旧 `#comment` / `#pi` pseudo-tag
    // + strip_non_element_stubs 二段構えは廃止。

    fn append_doctype_to_document(
        &self,
        _name: StrTendril,
        _public_id: StrTendril,
        _system_id: StrTendril,
    ) {
        // 現状 doctype node 化しない。quirks_mode は別 callback で通知される。
    }

    fn get_template_contents(&self, target: &usize) -> usize {
        // `create_element` が `flags.template=true`
        // を観測した時 `template_contents` slot に fragment root の arena index
        // を wire している。ここで返した index が html5ever の以降の
        // `TreeSink::append` の parent handle として使われる (template contents
        // は fragment root の子として積まれ、template element 自身は空の
        // children を保つ)。
        //
        // Defensive fallback: sink 経由でない直接組み立て (unit test 等) で
        // `template_contents` が未 wire な場合は旧挙動どおり `*target` を返す。
        // Node::is_in_document() gate は既存経路 (直接構築時の
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
/// `<template>` subtree は spec 上 inert なので skip する。
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
            // <template> 子孫 + 将来の inert subtree を統一 skip。
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

/// `<head>` 内で最初に現れる、非空 `href` 属性を持つ `<base>` element の
/// href 値を document order (tree order) で探す。見つからなければ `None`。
///
/// HTML Standard の "document base URL" algorithm
/// (§4.2.7 The base element / §urls-and-fetching "document base URL") は
/// "文書内で href 属性を**持つ**最初の `<base>` element" の frozen base URL
/// を document base URL とする — 値が空文字列であっても「href 属性を持つ」
/// 判定には数える。本実装はそれを厳密には満たせない:
/// `raikiri_traits::Element::attr` は空文字列の属性値を `None` に正規化する
/// 契約 (`collect_external_stylesheet_hrefs` の `disabled` 属性コメント参照)
/// のため、`href=""` を持つ `<base>` と href 属性自体が無い `<base>` をこの
/// trait surface からは区別できない (`has_attribute` 相当が無く、追加は
/// raikiri-traits の public surface 拡張になるため本変更の範囲外)。そのため
/// 厳密な spec 挙動 (`<base href=""><base
/// href="https://cdn.example/">` のような文書で、1 つ目の空 href base が
/// document base URL を確定させ 2 つ目が無視される) ではなく、非空 href を
/// 持つ最初の `<base>` を採用する — 実務上の文書ではほぼ同じ結果になる
/// (空 href の base 単体なら、どのみち fallback base URL への self-join に
/// 帰着し無視した場合と同じ URL になる)。
///
/// 探索範囲は `collect_external_stylesheet_hrefs` と同じ head-only DFS —
/// `<body>` 内の `<base>` は同じ理由で defer (今の scope では `<head>` 外の
/// stylesheet link 自体を扱っていないので、`<body>` 内 `<base>` を見ても
/// 適用対象が無い)。
///
/// href 値は trim する: `Element::attr` は truly-empty (`""`) のみ filter
/// する契約なので、空白のみの `href="   "` はここに届く。trim しないと
/// `resolve_url` の `Url::join("   ")` が (`collect_external_stylesheet_hrefs`
/// の link href コメントと同じ理由で) base URL 自身に解決されてしまい、
/// 「href 属性はあるが実質空」なケースを誤って override として扱う。
pub(crate) fn find_document_base_href(doc: &Document) -> Option<String> {
    use raikiri_traits::{Dom, Element, Node};

    let head_id = find_head_element(doc)?;

    let mut stack: Vec<raikiri_traits::NodeId> = vec![head_id];
    while let Some(id) = stack.pop() {
        let Some(node) = doc.node(id) else {
            // cov:ignore: unreachable in practice, mirrors the identical
            // defensive branch in `collect_external_stylesheet_hrefs` below.
            continue;
        };
        if !node.is_in_document() {
            continue;
        }
        if let Some(el) = node.as_element()
            && el.tag_name() == "base"
            && let Some(href) = el.attr("href")
        {
            let trimmed = href.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        let kids: Vec<_> = doc.child_ids(id).collect();
        for c in kids.into_iter().rev() {
            stack.push(c);
        }
    }
    None
}

/// `<head>` 内の `<link rel="stylesheet" href="...">` を document order で
/// 収集する。
///
/// `extract_inline_stylesheets` と同じ head-only DFS scope — `<body>` 内
/// `<link>` は `<style>` と同じ理由で defer。実際の fetch
/// (`NetworkProvider` 経由の I/O) はここでは行わない: `TreeSink::finish()`
/// (このモジュール) は I/O を持たない契約を保つ必要がある (sink 実装が
/// 観測可能な副作用を追加すると wall/sink 対象)。href の収集のみ行い、実
/// fetch は `ParseOptions::network` / `base_url` にアクセスできる
/// `parse.rs::parse_with_sink` 側の post-processing に委ねる。
///
/// `disabled` boolean attribute は判定しない: `raikiri_traits::Element::attr`
/// は値なし/空文字列 (`disabled` / `disabled=""` いずれも) を `None` に正規化
/// する契約 (`raikiri-dom/src/dom_impl.rs::ElementRef::attr` の
/// `.filter(|s| !s.is_empty())`) のため、属性の**値**ではなく**有無**を問う
/// boolean attribute はこの trait surface からは判別できない
/// (`has_attribute` 相当が無い)。追加は raikiri-traits の public surface
/// 変更 = wall/traits 対象であり、本 task 単独で unilateral に広げない
/// (follow-up は別途追跡する)。
///
/// ここで収集する href はまだ `<base>` element を反映していない生の
/// attribute 値: `<head>` 内の `<base href>` の探索・resolve は
/// [`find_document_base_href`] が別途担い、実際に "どの base URL に対して
/// href を解決するか" の合成は `parse.rs::fetch_external_stylesheets` が行う
/// (この関数自体は URL 解決を一切しない、収集のみの純粋関数のまま)。
///
/// `title` 属性付き `rel="alternate stylesheet"` の preferred/selected
/// stylesheet set semantics は未実装 — 非空 title を持つ alternate link を
/// 常に除外する挙動とその spec 上の根拠は `is_stylesheet_link` の doc
/// 参照。逆方向の残存する spec 逸脱もここに書いておく (`is_stylesheet_link`
/// 側ではカバーされていない点): 非 alternate な titled link (preferred
/// stylesheet) は preferred set の追跡が無いため常に適用してしまい、spec
/// 通りなら「複数の named stylesheet set のうち preferred set 以外は
/// 無効化する」べきところを無視している。この逸脱は `<link>` /
/// `<style>` 双方に共通する。
pub(crate) fn collect_external_stylesheet_hrefs(
    doc: &Document,
) -> Vec<(raikiri_traits::NodeId, String)> {
    use raikiri_traits::{Dom, Element, Node};

    let Some(head_id) = find_head_element(doc) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    let mut stack: Vec<raikiri_traits::NodeId> = vec![head_id];
    while let Some(id) = stack.pop() {
        // Flat `let...else continue` (rather than nesting the rest of the
        // loop body inside `if let Some(node) = doc.node(id) { ... }`, the
        // shape `extract_inline_stylesheets` above uses) — same intent, but
        // makes the never-taken `None` case an isolated one-line branch
        // instead of leaving an ambiguous coverage region on the enclosing
        // block's closing brace.
        let Some(node) = doc.node(id) else {
            // cov:ignore: unreachable in practice — every `id` pushed onto
            // `stack` comes from `find_head_element` or `doc.child_ids` for
            // this same `doc`, both of which only ever yield valid ids, so
            // `doc.node(id)` can't return `None` here. Kept as a `let...else`
            // (rather than `.expect(...)`) to match `Dom::node`'s documented
            // `Option` contract defensively, same as the sibling check this
            // mirrors in `extract_inline_stylesheets` above.
            continue;
        };
        if !node.is_in_document() {
            continue;
        }
        if let Some(el) = node.as_element()
            && el.tag_name() == "link"
            && is_stylesheet_link(el.attr("rel"), el.attr("type"), el.attr("title"))
            && let Some(href) = el.attr("href")
        {
            // `href`'s attribute type is "valid non-empty URL potentially
            // surrounded by spaces" — `Element::attr` only filters a truly
            // empty (`""`) value, so a whitespace-only `href="   "` still
            // reaches here. Trim before checking emptiness; without this,
            // `resolve_url`'s `Url::join("   ")` resolves to the
            // *page's own URL* (verified empirically against the `url`
            // crate), causing the page's own HTML to be fetched and fed to
            // the CSS parser as a stylesheet.
            let trimmed_href = href.trim();
            if !trimmed_href.is_empty() {
                out.push((id, trimmed_href.to_string()));
            }
        }
        let kids: Vec<_> = doc.child_ids(id).collect();
        for c in kids.into_iter().rev() {
            stack.push(c);
        }
    }
    out
}

/// `rel` トークンリストに `stylesheet` (ASCII case-insensitive) が含まれ、
/// かつ `type` 属性が無いか `text/css` (MIME パラメータを無視、ASCII
/// case-insensitive) の場合のみ true。HTML Standard §4.2.4 (The link
/// element) の外部 resource link 判定の該当部分のみを実装するサブセット —
/// `media` / `crossorigin` / `integrity` / `disabled` は現状 scope 外
/// (`collect_external_stylesheet_hrefs` doc 参照)。
///
/// `type` 属性の比較は `;` 以降 (MIME parameter、例:
/// `text/css; charset=utf-8`) を無視する — browser の実際の "type attribute
/// gate" 挙動 (MIME parameter は無視、essence のみ比較) に合わせる。
///
/// `rel` に `alternate` トークンも含み、かつ `title` 属性が非空の場合は
/// 無条件に false を返す (適用しない)。CSSOM "add a CSS style sheet"
/// (<https://drafts.csswg.org/cssom/#add-a-css-style-sheet>) の algorithm
/// では、この種のスタイルシートは以下によってのみ有効になる:
///
/// - step 4: alternate flag が unset かつ preferred stylesheet set name が
///   空文字列の場合に限り、そのシートの title で preferred set name を
///   書き換える (= alternate なシートは preferred set name の決定に関与
///   しない)。
/// - step 5: disabled flag を unset するのは、title が空文字列 / title が
///   preferred set name (last set name が null の場合) と一致 / title が
///   last (= user が選択した) set name と一致、のいずれか。
///
/// つまり非空 title を持つ alternate stylesheet が有効になるかどうかは、
/// 同一文書内の他の `<link>` の title (preferred set name の決定) や
/// user のスタイルシートセット選択 (last set name) 次第であり、この
/// crate には両方の追跡機構が一切無い。この関数は単一の `<link>` の
/// 属性だけを見る stateless な predicate なので、この判定を行う手段が
/// 無く、常に除外する — 「選択されている場合が spec 上あり得ない」の
/// ではなく、「選択されているかどうかをこの predicate の情報だけでは
/// 判定できない」が正確な理由。空 title の alternate stylesheet は
/// step 5 の第一分岐 (title が空文字列) により無条件で disabled flag が
/// unset されるので、従来通り適用対象のまま。
fn is_stylesheet_link(rel: Option<&str>, type_attr: Option<&str>, title: Option<&str>) -> bool {
    let Some(rel) = rel else {
        return false;
    };
    let mut has_stylesheet_token = false;
    let mut has_alternate_token = false;
    for token in rel.split_ascii_whitespace() {
        if token.eq_ignore_ascii_case("stylesheet") {
            has_stylesheet_token = true;
        } else if token.eq_ignore_ascii_case("alternate") {
            has_alternate_token = true;
        }
    }
    if !has_stylesheet_token {
        return false;
    }
    // Titled alternate stylesheet: excluded unconditionally, see doc
    // comment above. Checks non-emptiness directly (rather than relying on
    // `Element::attr`'s empty-string-to-`None` normalization at the sole
    // call site) so the predicate is self-consistent for any caller,
    // including this module's own unit tests below.
    if has_alternate_token && title.is_some_and(|t| !t.is_empty()) {
        return false;
    }
    type_attr.is_none_or(|t| {
        t.split(';')
            .next()
            .unwrap_or(t)
            .trim()
            .eq_ignore_ascii_case("text/css")
    })
}

/// side-table (`qual_names` / `attributes`) の内容を raikiri-dom::Node に写す。
///
/// - `qual_names`: element の namespace URI が HTML default (`ns!(html)`) 以外
///   なら `Node.namespace` に格納 (HTML default は `None` fast path のまま)。
/// - `attributes`: null-namespace attr のみ raikiri-dom に運ぶ (namespaced attr
///   = `xlink:href` on SVG 等は将来に defer)。`style` attr は `Node.inline_style`
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
            // null namespace 以外の attr (xlink:href 等) は現状 drop。SVG /
            // MathML full support は将来別途扱う。
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

#[cfg(test)]
mod stylesheet_link_tests {
    use super::is_stylesheet_link;

    #[test]
    fn no_rel_attribute_is_not_a_stylesheet_link() {
        assert!(!is_stylesheet_link(None, None, None));
    }

    #[test]
    fn rel_without_stylesheet_token_is_not_a_stylesheet_link() {
        assert!(!is_stylesheet_link(Some("icon"), None, None));
    }

    #[test]
    fn rel_stylesheet_token_is_case_insensitive() {
        assert!(is_stylesheet_link(Some("StyleSheet"), None, None));
        assert!(is_stylesheet_link(Some("STYLESHEET"), None, None));
    }

    #[test]
    fn rel_stylesheet_among_multiple_space_separated_tokens_matches() {
        assert!(is_stylesheet_link(Some("alternate stylesheet"), None, None));
        assert!(is_stylesheet_link(Some("stylesheet next"), None, None));
    }

    #[test]
    fn absent_type_attribute_is_treated_as_stylesheet() {
        assert!(is_stylesheet_link(Some("stylesheet"), None, None));
    }

    #[test]
    fn type_text_css_case_insensitive_is_treated_as_stylesheet() {
        assert!(is_stylesheet_link(
            Some("stylesheet"),
            Some("text/css"),
            None
        ));
        assert!(is_stylesheet_link(
            Some("stylesheet"),
            Some("Text/CSS"),
            None
        ));
    }

    #[test]
    fn non_css_type_attribute_is_not_treated_as_stylesheet() {
        assert!(!is_stylesheet_link(
            Some("stylesheet"),
            Some("application/rss+xml"),
            None
        ));
    }

    #[test]
    fn type_with_charset_mime_parameter_is_still_treated_as_stylesheet() {
        // browsers ignore MIME parameters (charset, etc.) when gating on the
        // `type` attribute's essence — only `text/css` (before any `;`)
        // matters.
        assert!(is_stylesheet_link(
            Some("stylesheet"),
            Some("text/css; charset=utf-8"),
            None
        ));
        assert!(is_stylesheet_link(
            Some("stylesheet"),
            Some("TEXT/CSS;charset=UTF-8"),
            None
        ));
    }

    #[test]
    fn non_css_essence_with_mime_parameter_is_not_treated_as_stylesheet() {
        assert!(!is_stylesheet_link(
            Some("stylesheet"),
            Some("application/rss+xml; charset=utf-8"),
            None
        ));
    }

    #[test]
    fn untitled_alternate_stylesheet_is_treated_as_stylesheet() {
        // CSSOM "add a CSS style sheet" step 5: an empty title unsets the
        // disabled flag unconditionally, regardless of the alternate flag.
        assert!(is_stylesheet_link(Some("alternate stylesheet"), None, None));
    }

    #[test]
    fn empty_string_title_on_alternate_stylesheet_is_treated_as_stylesheet() {
        // Same as the `None` case above, but exercises `Some("")` directly
        // rather than relying on the call site's `Element::attr` contract
        // (which normalizes an empty attribute value to `None` before this
        // function ever sees it) to collapse the two.
        assert!(is_stylesheet_link(
            Some("alternate stylesheet"),
            None,
            Some("")
        ));
    }

    #[test]
    fn titled_alternate_stylesheet_is_excluded() {
        // CSSOM "add a CSS style sheet" step 4-6: a titled alternate
        // stylesheet only has its disabled flag unset if it matches the
        // page's preferred/selected stylesheet set. This crate tracks
        // neither, so it can never legitimately be "selected" — excluded
        // unconditionally rather than applied as if always preferred.
        assert!(!is_stylesheet_link(
            Some("alternate stylesheet"),
            None,
            Some("High Contrast")
        ));
    }

    #[test]
    fn titled_alternate_stylesheet_is_excluded_regardless_of_type_match() {
        assert!(!is_stylesheet_link(
            Some("stylesheet alternate"),
            Some("text/css"),
            Some("High Contrast")
        ));
    }

    #[test]
    fn titled_non_alternate_stylesheet_link_still_applies() {
        // No `alternate` token in `rel` — a titled *non-alternate* link is
        // the preferred stylesheet (its title becomes the page's preferred
        // stylesheet set name, per CSSOM "add a CSS style sheet" step 4),
        // so it's unaffected by the alternate-only exclusion.
        assert!(is_stylesheet_link(
            Some("stylesheet"),
            None,
            Some("Default")
        ));
    }
}

#[cfg(test)]
mod collect_external_stylesheet_hrefs_tests {
    use super::collect_external_stylesheet_hrefs;
    use raikiri_dom::Document;

    #[test]
    fn document_without_a_head_element_yields_no_hrefs() {
        // find_head_element's None branch: a Document that never got a
        // <head> attached at all (html5ever's tree construction always
        // synthesizes one, so this only happens for a hand-built Document
        // like this one — exercised directly since collect_external_stylesheet_hrefs
        // is pub(crate) and doesn't need the full parse pipeline).
        let doc = Document::new();
        assert!(collect_external_stylesheet_hrefs(&doc).is_empty());
    }
}

#[cfg(test)]
mod find_document_base_href_tests {
    use super::find_document_base_href;
    use raikiri_dom::Document;

    #[test]
    fn document_without_a_head_element_yields_no_base_href() {
        // Mirrors collect_external_stylesheet_hrefs_tests's identical case:
        // find_head_element's None branch, only reachable via a hand-built
        // Document (html5ever's tree construction always synthesizes a
        // <head>). Full document-order / trim / empty-href / <template>
        // behavior is exercised at the `parse()` level in lib.rs, where a
        // mock `NetworkProvider` can observe which URL was actually
        // resolved and requested.
        let doc = Document::new();
        assert!(find_document_base_href(&doc).is_none());
    }
}
