//! raikiri-html — html5ever wrapper (`RaikiriTreeSink`) + [`parse`] /
//! [`parse_with_sink`] entrypoints。cascade 前の [`UncascadedDocument`] を produce
//! する薄い parser layer (責務は parse だけ、cascade / layout は含まない)。

mod parse;
mod sink;
mod types;

pub use parse::{parse, parse_with_sink};
pub use sink::RaikiriTreeSink;
pub use types::{ParseOptions, UncascadedDocument};

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
}
