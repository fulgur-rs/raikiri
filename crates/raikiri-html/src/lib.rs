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
}
