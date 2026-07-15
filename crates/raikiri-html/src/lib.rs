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

        // parse-only path: no stylesheet, no warnings
        assert!(uncascaded.stylesheet_sources.is_empty());
        assert!(uncascaded.warnings.is_empty());
    }
}
