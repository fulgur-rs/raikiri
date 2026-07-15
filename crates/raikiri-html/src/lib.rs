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
        assert_eq!(uncascaded.stylesheet_sources, vec![String::from("p{color:red}")]);
    }

    #[test]
    fn parse_ignores_link_stylesheet_in_m1_scope() {
        // M1: external <link rel="stylesheet"> は fetch せず stylesheet_sources
        // にも含めない。M2 network integration で `StylesheetSource::External`
        // に昇格予定。
        let html = br#"<html><head><link rel="stylesheet" href="foo.css"></head><body>x</body></html>"#;
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
            uncascaded.warnings.iter().map(|w| &w.kind).collect::<Vec<_>>()
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
        use std::borrow::Cow;
        use std::cell::Ref;
        use html5ever::interface::{
            Attribute, ElementFlags, NodeOrText, QualName, TreeSink,
        };
        use html5ever::tendril::StrTendril;
        use html5ever::tree_builder::QuirksMode;

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

            fn finish(self) -> UncascadedDocument { self.inner.finish() }
            fn parse_error(&self, msg: Cow<'static, str>) { self.inner.parse_error(msg) }
            fn get_document(&self) -> usize { self.inner.get_document() }
            fn elem_name<'a>(&'a self, t: &'a usize) -> Ref<'a, QualName> {
                self.inner.elem_name(t)
            }
            fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> usize {
                self.observed_elements.set(self.observed_elements.get() + 1);
                self.inner.create_element(name, attrs, flags)
            }
            fn create_comment(&self, text: StrTendril) -> usize { self.inner.create_comment(text) }
            fn create_pi(&self, target: StrTendril, data: StrTendril) -> usize {
                self.inner.create_pi(target, data)
            }
            fn append(&self, parent: &usize, child: NodeOrText<usize>) { self.inner.append(parent, child) }
            fn append_based_on_parent_node(&self, e: &usize, p: &usize, c: NodeOrText<usize>) {
                self.inner.append_based_on_parent_node(e, p, c)
            }
            fn append_doctype_to_document(&self, name: StrTendril, pid: StrTendril, sid: StrTendril) {
                self.inner.append_doctype_to_document(name, pid, sid)
            }
            fn get_template_contents(&self, t: &usize) -> usize { self.inner.get_template_contents(t) }
            fn same_node(&self, x: &usize, y: &usize) -> bool { self.inner.same_node(x, y) }
            fn set_quirks_mode(&self, mode: QuirksMode) { self.inner.set_quirks_mode(mode) }
            fn append_before_sibling(&self, s: &usize, n: NodeOrText<usize>) {
                self.inner.append_before_sibling(s, n)
            }
            fn add_attrs_if_missing(&self, t: &usize, a: Vec<Attribute>) {
                self.inner.add_attrs_if_missing(t, a)
            }
            fn remove_from_parent(&self, t: &usize) { self.inner.remove_from_parent(t) }
            fn reparent_children(&self, n: &usize, p: &usize) { self.inner.reparent_children(n, p) }
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
        assert_eq!(uncascaded.dom.node(kids[0]).unwrap().text_content(), Some("Hi"));
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
}
