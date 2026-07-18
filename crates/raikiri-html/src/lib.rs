//! raikiri-html — html5ever wrapper (`RaikiriTreeSink`) + [`parse`] /
//! [`parse_with_sink`] entrypoints。cascade 前の [`UncascadedDocument`] を produce
//! する薄い parser layer (責務は parse だけ、cascade / layout は含まない)。

mod parse;
mod sink;
mod types;
pub mod ua;

pub use parse::{parse, parse_with_sink};
pub use sink::RaikiriTreeSink;
pub use types::{ParseOptions, UncascadedDocument};
pub use ua::MINIMAL_UA_CSS;

#[cfg(test)]
#[allow(clippy::needless_lifetimes, clippy::collapsible_if)]
mod tests {
    use super::*;
    use raikiri_traits::{Dom, Element, Node, NodeKind};

    fn empty_options<'a>() -> ParseOptions<'a> {
        ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        }
    }

    fn find_first_by_tag<'a>(
        doc: &'a raikiri_dom::Document,
        target: &str,
    ) -> Option<raikiri_traits::NodeId> {
        fn recur(
            doc: &raikiri_dom::Document,
            id: raikiri_traits::NodeId,
            target: &str,
        ) -> Option<raikiri_traits::NodeId> {
            let node = doc.node(id)?;
            if let Some(el) = node.as_element() {
                if el.tag_name() == target {
                    return Some(id);
                }
            }
            for c in doc.child_ids(id) {
                if let Some(hit) = recur(doc, c, target) {
                    return Some(hit);
                }
            }
            None
        }
        recur(doc, doc.root_id(), target)
    }

    #[test]
    fn parse_hello_world_produces_expected_tree() {
        let html = b"<html><body><p>Hello</p></body></html>";
        let options = empty_options();
        let uncascaded = parse(&html[..], &options).expect("parse ok");

        // html > body > p > "Hello" の tree を assert。
        // 注意: html5ever tokenizer は text を複数 AppendText に分割する可能性
        // があるため、Text children を concat して assert する (M1 では
        // adjacent Text の auto-merge を実装しない spike 上限)。
        let p_id = find_first_by_tag(&uncascaded.dom, "p").expect("p element exists");
        let mut collected = String::new();
        for c in uncascaded.dom.child_ids(p_id) {
            let child = uncascaded.dom.node(c).unwrap();
            assert_eq!(
                child.kind(),
                NodeKind::Text,
                "expected only Text children under <p>, got {:?}",
                child.kind()
            );
            collected.push_str(child.text_content().unwrap_or(""));
        }
        assert_eq!(collected, "Hello");

        // parse-only path: no stylesheet
        assert!(uncascaded.stylesheet_sources.is_empty());
    }

    #[test]
    fn parse_extracts_style_element_content() {
        let html = b"<html><head><style>p{color:red}</style></head><body><p>x</p></body></html>";
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        assert_eq!(
            uncascaded.stylesheet_sources,
            vec![String::from("p{color:red}")]
        );
    }

    #[test]
    fn parse_ignores_link_stylesheet_in_m1_scope() {
        // M1: external <link rel="stylesheet"> は fetch せず stylesheet_sources
        // にも含めない。M2 network integration で `StylesheetSource::External`
        // に昇格予定。
        let html =
            br#"<html><head><link rel="stylesheet" href="foo.css"></head><body>x</body></html>"#;
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        assert!(
            uncascaded.stylesheet_sources.is_empty(),
            "external link stylesheets should be ignored in M1 scope"
        );
    }

    #[test]
    fn parse_records_html_parse_warning_on_malformed_input() {
        use raikiri_traits::WarningKind;

        // 明確に html5ever が非致命 parse error を報告する input:
        // </p> だけの closing tag は "unexpected end tag" を trigger する。
        let html = b"</p>";
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse recovers");

        assert!(
            uncascaded
                .warnings
                .iter()
                .any(|w| matches!(&w.kind, WarningKind::HtmlParseError { .. })),
            "expected at least one HtmlParseError warning, got: {:?}",
            uncascaded
                .warnings
                .iter()
                .map(|w| &w.kind)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn parse_returns_encoding_error_on_invalid_utf8() {
        use raikiri_traits::ParseError;

        let bad: &[u8] = &[0xFF, 0xFE, 0xFF, b'<', b'p', b'>'];
        let opts = empty_options();
        let err = parse(bad, &opts).expect_err("invalid utf-8 should fail");
        match err {
            ParseError::Encoding { label, reason } => {
                assert_eq!(label, "utf-8");
                assert!(!reason.is_empty(), "reason should be populated");
            }
            other => panic!("expected Encoding, got {other:?}"),
        }
    }

    #[test]
    fn parse_returns_io_error_on_read_failure() {
        use raikiri_traits::ParseError;
        use std::io::{Error, ErrorKind, Read};

        struct FailingReader;
        impl Read for FailingReader {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                Err(Error::other("boom"))
            }
        }

        let opts = empty_options();
        let err = parse(FailingReader, &opts).expect_err("read failure should fail");
        match err {
            ParseError::Io(inner) => {
                assert_eq!(inner.kind(), ErrorKind::Other);
                assert_eq!(inner.to_string(), "boom");
            }
            other => panic!("expected Io, got {other:?}"),
        }
    }

    #[test]
    fn parse_with_sink_via_transparent_wrapper() {
        use html5ever::interface::{Attribute, ElementFlags, NodeOrText, QualName, TreeSink};
        use html5ever::tendril::StrTendril;
        use html5ever::tree_builder::QuirksMode;
        use std::borrow::Cow;
        use std::cell::Ref;

        /// Consumer wrapper example: RaikiriTreeSink を丸ごと delegate するだけの
        /// 透過 sink。sanitize / rewrite の hook point としては何もしない。
        struct TransparentSink {
            inner: RaikiriTreeSink,
            observed_elements: std::cell::Cell<u32>,
        }

        impl TreeSink for TransparentSink {
            type Handle = usize;
            type Output = UncascadedDocument;
            type ElemName<'a> = Ref<'a, QualName>;

            fn finish(self) -> UncascadedDocument {
                self.inner.finish()
            }
            fn parse_error(&self, msg: Cow<'static, str>) {
                self.inner.parse_error(msg)
            }
            fn get_document(&self) -> usize {
                self.inner.get_document()
            }
            fn elem_name<'a>(&'a self, t: &'a usize) -> Ref<'a, QualName> {
                self.inner.elem_name(t)
            }
            fn create_element(
                &self,
                name: QualName,
                attrs: Vec<Attribute>,
                flags: ElementFlags,
            ) -> usize {
                self.observed_elements.set(self.observed_elements.get() + 1);
                self.inner.create_element(name, attrs, flags)
            }
            fn create_comment(&self, text: StrTendril) -> usize {
                self.inner.create_comment(text)
            }
            fn create_pi(&self, target: StrTendril, data: StrTendril) -> usize {
                self.inner.create_pi(target, data)
            }
            fn append(&self, parent: &usize, child: NodeOrText<usize>) {
                self.inner.append(parent, child)
            }
            fn append_based_on_parent_node(&self, e: &usize, p: &usize, c: NodeOrText<usize>) {
                self.inner.append_based_on_parent_node(e, p, c)
            }
            fn append_doctype_to_document(
                &self,
                name: StrTendril,
                pid: StrTendril,
                sid: StrTendril,
            ) {
                self.inner.append_doctype_to_document(name, pid, sid)
            }
            fn get_template_contents(&self, t: &usize) -> usize {
                self.inner.get_template_contents(t)
            }
            fn same_node(&self, x: &usize, y: &usize) -> bool {
                self.inner.same_node(x, y)
            }
            fn set_quirks_mode(&self, mode: QuirksMode) {
                self.inner.set_quirks_mode(mode)
            }
            fn append_before_sibling(&self, s: &usize, n: NodeOrText<usize>) {
                self.inner.append_before_sibling(s, n)
            }
            fn add_attrs_if_missing(&self, t: &usize, a: Vec<Attribute>) {
                self.inner.add_attrs_if_missing(t, a)
            }
            fn remove_from_parent(&self, t: &usize) {
                self.inner.remove_from_parent(t)
            }
            fn reparent_children(&self, n: &usize, p: &usize) {
                self.inner.reparent_children(n, p)
            }
            fn is_mathml_annotation_xml_integration_point(&self, h: &usize) -> bool {
                self.inner.is_mathml_annotation_xml_integration_point(h)
            }
        }

        let html = b"<html><body><p>Hi</p></body></html>";
        let opts = empty_options();
        let sink = TransparentSink {
            inner: RaikiriTreeSink::new(),
            observed_elements: std::cell::Cell::new(0),
        };
        // observed_elements is inside sink and moves into parse_with_sink.
        // We can't inspect it post-parse; instead we assert the returned Document
        // has the expected tree (proves finish() bubble ok).
        let uncascaded = parse_with_sink(&html[..], sink, &opts).expect("parse ok");
        let p_id = find_first_by_tag(&uncascaded.dom, "p").expect("p exists");
        let text = uncascaded.dom.node(p_id).unwrap();
        let kids: Vec<_> = uncascaded.dom.child_ids(p_id).collect();
        assert_eq!(kids.len(), 1);
        assert_eq!(
            uncascaded.dom.node(kids[0]).unwrap().text_content(),
            Some("Hi")
        );
        let _ = text;
    }

    #[test]
    fn parse_strips_comment_nodes_from_dom_tree() {
        use raikiri_traits::{Dom, Element, Node};

        let html = b"<html><body><!-- a comment --><p>hi</p></body></html>";
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");

        // walk the tree and verify no #comment tags remain
        fn scan(doc: &raikiri_dom::Document, id: raikiri_traits::NodeId) -> bool {
            if let Some(node) = doc.node(id)
                && let Some(el) = node.as_element()
                && matches!(el.tag_name(), "#comment" | "#pi")
            {
                return true;
            }
            for c in doc.child_ids(id) {
                if scan(doc, c) {
                    return true;
                }
            }
            false
        }
        assert!(
            !scan(&uncascaded.dom, uncascaded.dom.root_id()),
            "expected no #comment stub elements in the DOM tree after parse"
        );
    }

    #[test]
    fn parse_captures_quirks_mode_for_missing_doctype() {
        use raikiri_traits::QuirksMode;

        // Missing <!DOCTYPE html> triggers full quirks mode per HTML5 spec.
        let html = b"<html><body>x</body></html>";
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        assert_eq!(uncascaded.quirks_mode, QuirksMode::Quirks);
    }

    #[test]
    fn parse_captures_no_quirks_for_standards_doctype() {
        use raikiri_traits::QuirksMode;

        let html = b"<!DOCTYPE html><html><body>x</body></html>";
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        assert_eq!(uncascaded.quirks_mode, QuirksMode::NoQuirks);
    }

    #[test]
    fn parse_survives_table_foster_parenting() {
        // <table> 直下 text の foster parenting は html5ever が
        // append_before_sibling(AppendText(...)) を trigger する典型 case。
        // panic せず parse が完走することのみ verify。
        let html = b"<table>stray text<tr><td>x</td></tr></table>";
        let opts = empty_options();
        let _ = parse(&html[..], &opts).expect("parse should not panic");
    }

    #[test]
    fn parse_survives_deeply_nested_html() {
        // Build ~5000 nested <div> — recursive walker would stack overflow.
        // Iterative walker completes fine.
        let mut html = String::new();
        let depth = 5000;
        for _ in 0..depth {
            html.push_str("<div>");
        }
        html.push_str("hello");
        for _ in 0..depth {
            html.push_str("</div>");
        }
        let opts = empty_options();
        let _ = parse(html.as_bytes(), &opts).expect("parse ok — walkers must handle deep nesting");
    }

    #[test]
    fn parse_maps_reader_invaliddata_to_io_not_encoding() {
        use raikiri_traits::ParseError;
        use std::io::{Error, ErrorKind, Read};

        /// Reader that returns InvalidData with an I/O-shaped message
        /// (not a UTF-8 error). Under the old read_to_string mapping this
        /// would be misclassified as ParseError::Encoding; the new
        /// read_to_end + from_utf8 path correctly reports ParseError::Io.
        struct BadKindReader;
        impl Read for BadKindReader {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                Err(Error::new(ErrorKind::InvalidData, "disk sector unreadable"))
            }
        }

        let opts = empty_options();
        let err = parse(BadKindReader, &opts).expect_err("reader failure should error");
        assert!(
            matches!(err, ParseError::Io(_)),
            "InvalidData from reader must map to ParseError::Io, not Encoding (got {err:?})"
        );
    }

    #[test]
    fn parse_extracts_multiple_style_blocks_in_document_order() {
        // Multiple <style> blocks — verify source-order preserved so cascade
        // tie-breaking (equal specificity → last-wins) works correctly.
        let html = b"<html><head>\
                     <style>p{color:red}</style>\
                     <style>p{color:blue}</style>\
                     <style>p{color:green}</style>\
                     </head><body><p>x</p></body></html>";
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        assert_eq!(
            uncascaded.stylesheet_sources,
            vec![
                String::from("p{color:red}"),
                String::from("p{color:blue}"),
                String::from("p{color:green}"),
            ]
        );
    }

    #[test]
    fn parse_skips_style_inside_template_element() {
        // <template> contents are inert per spec — <style> inside must not
        // appear in stylesheet_sources. Minimum m1.3 fix (skip template subtree
        // during extraction). Full template-fragment isolation tracked as bd-xno.
        let html = b"<html><head>\
                     <template><style>p{color:red}</style></template>\
                     <style>p{color:blue}</style>\
                     </head><body><p>x</p></body></html>";
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        // Only the top-level <style> should appear.
        assert_eq!(
            uncascaded.stylesheet_sources,
            vec![String::from("p{color:blue}")]
        );
    }

    #[test]
    fn parse_marks_template_descendants_out_of_document() {
        // raikiri-spike-37c: <template> element 自身は flat tree の一員なので
        // is_in_document()=true、その descendants (子孫の element / text) は
        // false であることを parse 経路の bit populate で pin する。
        //
        // 現在の sink には mark_in_document_flags phase が無いため、default
        // true が clear されず descendant も true になる → 失敗する failing test。
        let html = b"<html><head></head><body>\
                     <template><p id=\"inner\">hi</p></template>\
                     </body></html>";
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        let doc = &uncascaded.dom;

        let mut saw_template = false;
        let mut saw_inner_p = false;
        let mut saw_inner_text = false;
        for id_u in 0..doc.node_count() {
            let id = raikiri_traits::NodeId::new(id_u as u64);
            let n = doc.node(id).expect("in-range");
            if let Some(el) = n.as_element() {
                match el.tag_name() {
                    "template" => {
                        assert!(
                            n.is_in_document(),
                            "template element itself must be in document"
                        );
                        saw_template = true;
                    }
                    "p" if el.id() == Some("inner") => {
                        assert!(
                            !n.is_in_document(),
                            "<p> inside <template> must be out of document"
                        );
                        saw_inner_p = true;
                    }
                    _ => {}
                }
            }
            if n.text_content() == Some("hi") {
                assert!(
                    !n.is_in_document(),
                    "text inside <template> must be out of document"
                );
                saw_inner_text = true;
            }
        }
        assert!(saw_template, "template element should exist in parsed tree");
        assert!(
            saw_inner_p,
            "<p id=inner> should exist inside template subtree"
        );
        assert!(
            saw_inner_text,
            "'hi' text should exist inside template subtree"
        );
    }

    #[test]
    fn parse_marks_body_children_in_document() {
        // raikiri-spike-37c: normal HTML (template 無し) を parse すると全 node が
        // is_in_document()=true。default true が保たれる regression pin。
        let html = b"<html><head></head><body><p>hi</p></body></html>";
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        let doc = &uncascaded.dom;
        for id_u in 0..doc.node_count() {
            let id = raikiri_traits::NodeId::new(id_u as u64);
            let n = doc.node(id).expect("in-range");
            assert!(
                n.is_in_document(),
                "node {} ({:?}) expected in_document",
                id_u,
                n.kind()
            );
        }
    }

    #[test]
    fn parse_marks_nested_template_descendants_out_of_document() {
        // raikiri-spike-37c: 深いネスト (template > div > span > text) でも
        // in_document bit が subtree 全体に伝播する。single-pass DFS で
        // in_template state が正しく引き継がれることを pin。
        let html = b"<html><body>\
                     <template><div><span>x</span></div></template>\
                     </body></html>";
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        let doc = &uncascaded.dom;
        for id_u in 0..doc.node_count() {
            let id = raikiri_traits::NodeId::new(id_u as u64);
            let n = doc.node(id).expect("in-range");
            if let Some(el) = n.as_element() {
                match el.tag_name() {
                    "div" | "span" => assert!(
                        !n.is_in_document(),
                        "<{}> inside <template> must be out of document",
                        el.tag_name()
                    ),
                    _ => {}
                }
            }
            if n.text_content() == Some("x") {
                assert!(
                    !n.is_in_document(),
                    "text 'x' inside <template> must be out of document"
                );
            }
        }
    }

    #[test]
    fn parse_then_cascade_skips_template_descendants() {
        // raikiri-spike-37c: cascade が template subtree を skip する silent bug fix
        // regression pin。詳細な cascaded map の shape reflection は raikiri-style
        // 内部の unit test で担保するのが正道 (未存在なら Task 4 で追加)、この
        // integration test は "parse → cascade の chain が template 内 element を
        // 触っても error / panic しない" ことと、bit populate が cascade 呼び出し
        // 前後で保たれることを pin する。
        //
        // 追加 pin (roborev-equivalent advisor 指摘): resolve_inheritance の
        // is_in_document() gate 実装ミスは `cascade.computed.len() ==
        // dom.node_count()` という m1.23 contract (raikiri/src/lib.rs
        // `html_document_cascade_populated_after_construct` が非-template
        // document でのみ pin していた) を template を含む document で破り得る
        // — raikiri-dom::layout::preshape_text / raikiri-paint::text::draw_text_node
        // は node_id で `cascade.computed[idx]` に直接 index するため、破れると
        // OOB panic に繋がる。TestDoc 経由の raikiri-style 内部 unit test は
        // is_in_document() が常に true な default 実装のため、この contract
        // 破れを検出できない (parse 経由で実際に bit が false になる document
        // でのみ再現する) — 本 integration test がそのカバレッジを担う。
        let html = b"<html><head><style>p { color: red }</style></head><body>\
                     <p>outer</p>\
                     <template><p id=\"inner\">inner</p></template>\
                     </body></html>";
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        let tree = raikiri_style::build_rule_tree(&uncascaded.dom);
        let cascade = raikiri_style::cascade(&uncascaded.dom, &tree)
            .expect("cascade must not error / panic on template subtree");

        // m1.23 contract: cascade.computed.len() == dom.node_count() でなければ
        // ならない — たとえ template 子孫が cascade gate で skip されても、
        // index 契約 (raikiri-dom / raikiri-paint が node_id で直接 index) を
        // 破ってはいけない。
        assert_eq!(
            cascade.computed.len(),
            uncascaded.dom.node_count(),
            "cascade.computed.len() must equal node_count() even with template descendants (m1.23 contract)"
        );

        // cascade 呼び出し後も inner <p> は out-of-document のまま (cascade が bit
        // を触ることは無いという contract の pin)。
        //
        // roborev job 293 L1 finding: 加えて outer <p> と inner <p> の ComputedValues
        // を実際に検証する。gate が動いていれば outer には `p { color: red }` rule
        // が適用され CssColor { r:255, g:0, b:0 } となり、inner には rule が適用
        // されず initial (CssColor::BLACK = { r:0, g:0, b:0 }) が残る。もし cascade
        // gate を両方削除したら inner にも red rule が届き BLACK ではなくなるため、
        // この assert 対で gate 動作が本当に発火していることを pin する。
        use raikiri_style::property::CssColor;
        const RED: CssColor = CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        };

        let mut outer_p_id: Option<usize> = None;
        let mut inner_p_id: Option<usize> = None;
        for id_u in 0..uncascaded.dom.node_count() {
            let id = raikiri_traits::NodeId::new(id_u as u64);
            let n = uncascaded.dom.node(id).unwrap();
            if let Some(el) = n.as_element()
                && el.tag_name() == "p"
            {
                if el.id() == Some("inner") {
                    assert!(
                        !n.is_in_document(),
                        "inner <p> should remain out of document after cascade"
                    );
                    inner_p_id = Some(id_u);
                } else {
                    outer_p_id = Some(id_u);
                }
            }
        }
        let outer_p_id = outer_p_id.expect("outer <p> should exist");
        let inner_p_id = inner_p_id.expect("<p id=inner> should exist inside template");

        // Index into cascade.computed for both nodes must not panic (proves
        // out[idx] was still written for the inert node despite the gate).
        let outer_cv = &cascade.computed[outer_p_id];
        let inner_cv = &cascade.computed[inner_p_id];

        assert_eq!(
            outer_cv.color, RED,
            "outer <p> should have red rule applied (in-document, rule matches)"
        );
        assert_eq!(
            inner_cv.color,
            CssColor::BLACK,
            "inner <p> should keep initial color (cascade gate skips template descendants)"
        );
    }

    #[test]
    fn parse_ignores_body_style_in_m1_scope() {
        // 設計仕様書 §6 MVP: <head> 内 <style> のみ登録。<body> 内 <style> は
        // position-aware semantics を要するため defer。
        let html = b"<html><head><style>p{color:red}</style></head>\
                     <body><style>p{color:blue}</style><p>x</p></body></html>";
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        // Only <head> style should be extracted.
        assert_eq!(
            uncascaded.stylesheet_sources,
            vec![String::from("p{color:red}")]
        );
    }

    // ── Attribute / namespace wiring (raikiri-spike-blg) ────────

    #[test]
    fn parse_wires_style_attribute_to_inline_style_source() {
        // real HTML `<p style="color:red">` → cascade が消費できる
        // inline_style_source が populate される (end-to-end verify)。
        let html = b"<html><body><p style=\"color:red\">Hi</p></body></html>";
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        let p_id = find_first_by_tag(&uncascaded.dom, "p").expect("p exists");
        let p_node = uncascaded.dom.node(p_id).expect("p node exists");
        let p_elem = p_node.as_element().expect("p is element");
        assert_eq!(p_elem.inline_style_source(), Some("color:red"));
        // `style` は attributes には積まれない (Node.inline_style 側に分離)。
        assert_eq!(p_elem.attr("style"), Some("color:red"));
    }

    #[test]
    fn parse_wires_id_class_and_data_attributes() {
        let html =
            br#"<html><body><div id="main" class="foo bar baz" data-x="42"></div></body></html>"#;
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        let div_id = find_first_by_tag(&uncascaded.dom, "div").expect("div exists");
        let div_node = uncascaded.dom.node(div_id).expect("div node exists");
        let div = div_node.as_element().expect("div is element");
        assert_eq!(div.id(), Some("main"));
        assert!(div.has_class("foo"));
        assert!(div.has_class("bar"));
        assert!(div.has_class("baz"));
        assert!(!div.has_class("qux"));
        assert_eq!(div.attr("data-x"), Some("42"));
    }

    #[test]
    fn parse_treats_html_elements_namespace_uri_as_none() {
        let html = b"<html><body><p>hi</p></body></html>";
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        let p_id = find_first_by_tag(&uncascaded.dom, "p").expect("p exists");
        let p_node = uncascaded.dom.node(p_id).expect("p node exists");
        let p = p_node.as_element().expect("p is element");
        // HTML default namespace は fast path として None を返す。
        assert_eq!(p.namespace_uri(), None);
    }

    #[test]
    fn parse_wires_svg_namespace_uri() {
        // html5ever は <svg> 内 element を automatically SVG namespace に置く。
        let html = br#"<html><body><svg><g></g></svg></body></html>"#;
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        let svg_id = find_first_by_tag(&uncascaded.dom, "svg").expect("svg exists");
        let g_id = find_first_by_tag(&uncascaded.dom, "g").expect("g exists");
        let svg_node = uncascaded.dom.node(svg_id).expect("svg node exists");
        let svg = svg_node.as_element().expect("svg is element");
        assert_eq!(svg.namespace_uri(), Some("http://www.w3.org/2000/svg"));
        let g_node = uncascaded.dom.node(g_id).expect("g node exists");
        let g = g_node.as_element().expect("g is element");
        assert_eq!(g.namespace_uri(), Some("http://www.w3.org/2000/svg"));
    }

    #[test]
    fn parse_missing_style_attribute_leaves_inline_style_none() {
        let html = b"<html><body><p>x</p></body></html>";
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        let p_id = find_first_by_tag(&uncascaded.dom, "p").expect("p exists");
        let p_node = uncascaded.dom.node(p_id).expect("p node exists");
        let p = p_node.as_element().expect("p is element");
        assert_eq!(p.inline_style_source(), None);
    }

    #[test]
    fn parse_empty_style_attribute_normalizes_to_none() {
        // trait contract: `style=""` は inline_style_source が None。
        let html = br#"<html><body><p style=""></p></body></html>"#;
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        let p_id = find_first_by_tag(&uncascaded.dom, "p").expect("p exists");
        let p_node = uncascaded.dom.node(p_id).expect("p node exists");
        let p = p_node.as_element().expect("p is element");
        assert_eq!(p.inline_style_source(), None);
    }

    #[test]
    fn sink_first_wins_on_duplicate_style_attribute() {
        // Defensive: html5ever は tokenizer 段で duplicate attr を dedupe する
        // が (§13.2.5.32)、raikiri-html sink 単体が受け取る Vec<Attribute> が
        // duplicate を含む可能性を排除しない (external consumer が TreeSink を
        // wrap して重複 attr を注入する scenario も含む)。この test は sink
        // 単体を driver に見立てて "style を 2 回渡すと最初 (color:red) が勝つ"
        // 挙動を pin する。
        use html5ever::interface::{Attribute, ElementFlags, QualName, TreeSink};
        use html5ever::tendril::StrTendril;
        use markup5ever::{LocalName, Namespace};

        let sink = RaikiriTreeSink::new();
        let name = QualName::new(
            None,
            Namespace::from("http://www.w3.org/1999/xhtml"),
            LocalName::from("p"),
        );
        let attr = |v: &str| Attribute {
            name: QualName::new(None, Namespace::from(""), LocalName::from("style")),
            value: StrTendril::from(v),
        };
        // sink に "style=color:red" と "style=color:blue" を順に渡す。
        let attrs = vec![attr("color:red"), attr("color:blue")];
        let idx = sink.create_element(name, attrs, ElementFlags::default());
        // Document の Handle は root。attach しないと finish 前に見つからないため
        // append 経由で root child にする。
        sink.append(
            &sink.get_document(),
            html5ever::interface::NodeOrText::AppendNode(idx),
        );
        let uncascaded = sink.finish();

        let p_id = find_first_by_tag(&uncascaded.dom, "p").expect("p exists");
        let p_node = uncascaded.dom.node(p_id).expect("p node exists");
        let p = p_node.as_element().expect("p is element");
        // first-wins (regression pin for wire_side_tables)。
        assert_eq!(p.inline_style_source(), Some("color:red"));
    }

    // ── MathML annotation-xml integration point (raikiri-spike-eil) ─────
    //
    // HTML5 §13.2.5 tree construction: MathML `annotation-xml` element は
    // `encoding` attribute の value が ASCII case-insensitive で `text/html`
    // または `application/xhtml+xml` と一致するときに HTML integration point
    // となる。tree construction algorithm の branch 判定に使う中心 predicate。

    /// Helper: annotation-xml element を指定 namespace + encoding で作成し、
    /// `is_mathml_annotation_xml_integration_point` の返り値を返す。
    fn probe_annotation_xml_integration_point(
        ns_uri: &str,
        local: &str,
        encoding: Option<&str>,
    ) -> bool {
        use html5ever::interface::{Attribute, ElementFlags, QualName, TreeSink};
        use html5ever::tendril::StrTendril;
        use markup5ever::{LocalName, Namespace};

        let sink = RaikiriTreeSink::new();
        let name = QualName::new(None, Namespace::from(ns_uri), LocalName::from(local));
        let attrs = encoding
            .map(|v| {
                vec![Attribute {
                    name: QualName::new(None, Namespace::from(""), LocalName::from("encoding")),
                    value: StrTendril::from(v),
                }]
            })
            .unwrap_or_default();
        let idx = sink.create_element(name, attrs, ElementFlags::default());
        sink.is_mathml_annotation_xml_integration_point(&idx)
    }

    #[test]
    fn annotation_xml_with_text_html_encoding_is_integration_point() {
        assert!(probe_annotation_xml_integration_point(
            "http://www.w3.org/1998/Math/MathML",
            "annotation-xml",
            Some("text/html"),
        ));
    }

    #[test]
    fn annotation_xml_encoding_comparison_is_ascii_case_insensitive() {
        // spec: "ASCII case-insensitive match"
        for v in [
            "Text/HTML",
            "TEXT/HTML",
            "text/HTML",
            "Application/XHTML+XML",
        ] {
            assert!(
                probe_annotation_xml_integration_point(
                    "http://www.w3.org/1998/Math/MathML",
                    "annotation-xml",
                    Some(v),
                ),
                "encoding={v:?} should match (ASCII case-insensitive)"
            );
        }
    }

    #[test]
    fn annotation_xml_with_application_xhtml_xml_encoding_is_integration_point() {
        assert!(probe_annotation_xml_integration_point(
            "http://www.w3.org/1998/Math/MathML",
            "annotation-xml",
            Some("application/xhtml+xml"),
        ));
    }

    #[test]
    fn annotation_xml_with_unrelated_encoding_is_not_integration_point() {
        // spec は text/html と application/xhtml+xml の 2 種のみ integration point。
        // application/xml / image/svg+xml / 空文字列 は non-match。
        for v in ["application/xml", "image/svg+xml", "", "text/plain"] {
            assert!(
                !probe_annotation_xml_integration_point(
                    "http://www.w3.org/1998/Math/MathML",
                    "annotation-xml",
                    Some(v),
                ),
                "encoding={v:?} should not match"
            );
        }
    }

    #[test]
    fn annotation_xml_without_encoding_attribute_is_not_integration_point() {
        assert!(!probe_annotation_xml_integration_point(
            "http://www.w3.org/1998/Math/MathML",
            "annotation-xml",
            None,
        ));
    }

    #[test]
    fn non_mathml_annotation_xml_is_not_integration_point() {
        // Defense in depth: annotation-xml でも MathML namespace 以外では
        // integration point ではない (spec の主語が "MathML annotation-xml")。
        assert!(!probe_annotation_xml_integration_point(
            "http://www.w3.org/2000/svg",
            "annotation-xml",
            Some("text/html"),
        ));
        assert!(!probe_annotation_xml_integration_point(
            "http://www.w3.org/1999/xhtml",
            "annotation-xml",
            Some("text/html"),
        ));
    }

    #[test]
    fn non_annotation_xml_mathml_element_is_not_integration_point() {
        // annotation-xml 以外の MathML 要素は integration point ではない。
        assert!(!probe_annotation_xml_integration_point(
            "http://www.w3.org/1998/Math/MathML",
            "mi",
            Some("text/html"),
        ));
    }

    #[test]
    fn annotation_xml_duplicate_encoding_attribute_uses_first_value() {
        // HTML §13.2.5.32 duplicate attribute → ignore later occurrences。
        // sink_first_wins_on_duplicate_style_attribute と同じ first-wins 契約を
        // integration point 判定でも守る (later match が earlier non-match を
        // 上書きしないことを pin する)。
        use html5ever::interface::{Attribute, ElementFlags, QualName, TreeSink};
        use html5ever::tendril::StrTendril;
        use markup5ever::{LocalName, Namespace};

        let encoding_attr = |v: &str| Attribute {
            name: QualName::new(None, Namespace::from(""), LocalName::from("encoding")),
            value: StrTendril::from(v),
        };

        // Case A: first=text/html (match), second=application/xml (non-match)
        // → first wins → integration point (true)
        {
            let sink = RaikiriTreeSink::new();
            let name = QualName::new(
                None,
                Namespace::from("http://www.w3.org/1998/Math/MathML"),
                LocalName::from("annotation-xml"),
            );
            let idx = sink.create_element(
                name,
                vec![encoding_attr("text/html"), encoding_attr("application/xml")],
                ElementFlags::default(),
            );
            assert!(
                sink.is_mathml_annotation_xml_integration_point(&idx),
                "first encoding=text/html should win over later encoding=application/xml"
            );
        }

        // Case B: first=application/xml (non-match), second=text/html (match)
        // → first wins → NOT integration point (false)
        {
            let sink = RaikiriTreeSink::new();
            let name = QualName::new(
                None,
                Namespace::from("http://www.w3.org/1998/Math/MathML"),
                LocalName::from("annotation-xml"),
            );
            let idx = sink.create_element(
                name,
                vec![encoding_attr("application/xml"), encoding_attr("text/html")],
                ElementFlags::default(),
            );
            assert!(
                !sink.is_mathml_annotation_xml_integration_point(&idx),
                "later encoding=text/html must not override earlier non-match"
            );
        }
    }

    // End-to-end parse: annotation-xml integration point の判定は tree
    // construction algorithm の branch を切り替えるので、parse 経由でも
    // "子要素の namespace が予想通りか" で観測できる。
    // - encoding=text/html → HTML integration point 発動 → 子は HTML namespace
    //   (raikiri-dom fast path で namespace_uri() = None)
    // - encoding 不在 → 通常の MathML foreign content → 子は MathML namespace

    #[test]
    fn parse_annotation_xml_integration_point_inherits_html_namespace_for_children() {
        // annotation-xml encoding=text/html は HTML integration point。中の
        // 未知要素 <foo> は HTML namespace として解釈されるべき。
        let html =
            br#"<math><annotation-xml encoding="text/html"><foo>x</foo></annotation-xml></math>"#;
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        let foo_id = find_first_by_tag(&uncascaded.dom, "foo").expect("foo exists");
        let foo_node = uncascaded.dom.node(foo_id).expect("foo node exists");
        let foo = foo_node.as_element().expect("foo is element");
        assert_eq!(
            foo.namespace_uri(),
            None,
            "child inside HTML integration point should be HTML (None fast path)"
        );
    }

    #[test]
    fn parse_annotation_xml_non_integration_wraps_children_in_mathml_namespace() {
        // annotation-xml (encoding 不在) は integration point ではない。中の
        // 未知要素 <foo> は MathML foreign content として MathML namespace で解釈される。
        let html = br#"<math><annotation-xml><foo>x</foo></annotation-xml></math>"#;
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        let foo_id = find_first_by_tag(&uncascaded.dom, "foo").expect("foo exists");
        let foo_node = uncascaded.dom.node(foo_id).expect("foo node exists");
        let foo = foo_node.as_element().expect("foo is element");
        assert_eq!(
            foo.namespace_uri(),
            Some("http://www.w3.org/1998/Math/MathML"),
            "child inside non-integration MathML should stay in MathML namespace"
        );
    }

    #[test]
    fn parse_strips_many_comments_under_one_parent() {
        use raikiri_traits::{Dom, Element, Node};

        // 100 comments under body — verifies retain_children handles bulk correctly.
        let mut html = String::from("<html><head></head><body>");
        for i in 0..100 {
            html.push_str(&format!("<!-- comment {i} -->"));
        }
        html.push_str("<p>x</p></body></html>");
        let opts = empty_options();
        let uncascaded = parse(html.as_bytes(), &opts).expect("parse ok");

        // No #comment / #pi should remain in the tree.
        fn scan(doc: &raikiri_dom::Document, id: raikiri_traits::NodeId) -> bool {
            if let Some(node) = doc.node(id)
                && let Some(el) = node.as_element()
                && matches!(el.tag_name(), "#comment" | "#pi")
            {
                return true;
            }
            for c in doc.child_ids(id) {
                if scan(doc, c) {
                    return true;
                }
            }
            false
        }
        assert!(
            !scan(&uncascaded.dom, uncascaded.dom.root_id()),
            "100 comment stubs must all be stripped"
        );
    }

    // ── UA CSS bundle (M1.4a、raikiri-spike-m1.22) ──────────────

    #[test]
    fn minimal_ua_css_covers_required_display_block_selectors() {
        // spec §M1.4a Scope: html, body, div, p, h1-h6 が display: block を持つ。
        //
        // 各 tag について、rule 行の存在を検査する — 「行を trim_start した後
        // `{tag}` で始まり、その直後 whitespace を挟んで `{` が来る」ケースだけ
        // 選択子と扱う。素の contains() だと `p` が comment 内の `Appendix` /
        // `paragraph` / `display` の一部に match してしまう (roborev job 217 low
        // 対応)。
        for tag in [
            "html", "body", "div", "p", "h1", "h2", "h3", "h4", "h5", "h6",
        ] {
            let has_rule = MINIMAL_UA_CSS.lines().any(|line| {
                let trimmed = line.trim_start();
                trimmed
                    .strip_prefix(tag)
                    .map(|rest| rest.trim_start().starts_with('{'))
                    .unwrap_or(false)
            });
            assert!(
                has_rule,
                "MINIMAL_UA_CSS is missing selector rule `{tag} {{ … }}`",
            );
        }
        // spec 参照コメントが含まれていること (CSS 2.1 App.D 由来の cleanroom 印)
        assert!(
            MINIMAL_UA_CSS.contains("CSS 2.1 App.D"),
            "MINIMAL_UA_CSS should contain spec reference comments",
        );
        // display: block declaration が含まれていること (直接文字列で確認)
        assert!(
            MINIMAL_UA_CSS.contains("display: block"),
            "MINIMAL_UA_CSS should declare display: block",
        );
    }

    #[test]
    fn parse_injects_default_ua_stylesheet_into_document() {
        use raikiri_traits::StylesheetKind;

        let html = b"<html><body><p>Hi</p></body></html>";
        let opts = empty_options();
        let doc = parse(&html[..], &opts).expect("parse ok");

        let ua_entries: Vec<&str> = doc
            .dom
            .stylesheets()
            .filter(|(_, k)| *k == StylesheetKind::UserAgent)
            .map(|(s, _)| s)
            .collect();
        assert_eq!(ua_entries.len(), 1, "exactly one UA CSS entry expected");
        assert_eq!(ua_entries[0], MINIMAL_UA_CSS);
    }

    #[test]
    fn parse_injects_extra_stylesheets_as_author() {
        use raikiri_traits::StylesheetKind;

        let html = b"<html><body></body></html>";
        let extra_a = "a { color: red }";
        let extra_b = "b { color: blue }";
        let opts = ParseOptions {
            extra_stylesheets: &[extra_a, extra_b],
            network: None,
            base_url: None,
        };
        let doc = parse(&html[..], &opts).expect("parse ok");

        let author_entries: Vec<&str> = doc
            .dom
            .stylesheets()
            .filter(|(_, k)| *k == StylesheetKind::Author)
            .map(|(s, _)| s)
            .collect();
        assert_eq!(author_entries.len(), 2);
        assert_eq!(author_entries[0], extra_a);
        assert_eq!(author_entries[1], extra_b);
    }
}
