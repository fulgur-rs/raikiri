use super::*;

#[cfg(test)]
mod flags_tests {
    use super::*;

    #[test]
    fn node_flags_default_is_empty() {
        let f = NodeFlags::default();
        assert!(!f.contains(NodeFlags::IS_IN_DOCUMENT));
    }

    #[test]
    fn node_new_document_has_is_in_document_set_by_default() {
        // Node::new_document() creates the Document root, always in the flat tree.
        let n = Node::new_document();
        assert!(n.is_in_document());
    }

    #[test]
    fn node_new_element_has_is_in_document_set_by_default() {
        let n = Node::new_element(SmolStr::new("p"), taffy::Style::default(), None);
        assert!(n.is_in_document());
    }

    #[test]
    fn node_new_text_has_is_in_document_set_by_default() {
        let n = Node::new_text(SmolStr::new("hi"));
        assert!(n.is_in_document());
    }

    #[test]
    fn set_in_document_toggles_bit() {
        let mut n = Node::new_document();
        n.set_in_document(false);
        assert!(!n.is_in_document());
        n.set_in_document(true);
        assert!(n.is_in_document());
    }

    #[test]
    fn node_new_comment_kind_and_default_flag_state() {
        // A Comment constructor has kind = NodeKind::Comment,
        // with IS_IN_DOCUMENT=true by default (an optimistic initial value
        // cleared later by mark_in_document_flags, as with Element / Text).
        let n = Node::new_comment(SmolStr::new("hello"));
        assert_eq!(n.kind(), NodeKind::Comment);
        assert!(
            n.is_in_document(),
            "constructor default follows Element/Text pattern"
        );
        // tag_name returns None for a Comment (not an Element).
        assert_eq!(n.tag_name(), None);
        // text_layout returns None for a Comment (not Text).
        assert!(n.text_layout().is_none());
    }

    #[test]
    fn node_new_processing_instruction_kind_and_default_flag_state() {
        let n = Node::new_processing_instruction(
            SmolStr::new("xml-stylesheet"),
            SmolStr::new("href='x.css'"),
        );
        assert_eq!(n.kind(), NodeKind::ProcessingInstruction);
        assert!(n.is_in_document());
        assert_eq!(n.tag_name(), None);
        assert!(n.text_layout().is_none());
    }

    #[test]
    fn node_new_document_fragment_kind_and_default_flag_state() {
        let n = Node::new_document_fragment();
        assert_eq!(n.kind(), NodeKind::DocumentFragment);
        // The Fragment root starts detached when used, so it becomes false after
        // marking; the constructor itself defaults to true.
        assert!(n.is_in_document());
        assert_eq!(n.tag_name(), None);
        assert!(n.text_layout().is_none());
    }

    #[test]
    fn node_flags_bit_values_match_blitz_raw() {
        // Regression check: blitz `NodeFlags` must match the raw bit values
        // defined in blitz-dom/src/node/node.rs:50-58:
        //   IS_INLINE_ROOT = 0b001, IS_TABLE_ROOT = 0b010, IS_IN_DOCUMENT = 0b100
        //
        // This makes future blitz-compat conversion a trivial
        // `NodeFlags::from_bits(x)` cast. Add future bits at the same positions
        // as blitz.
        assert_eq!(NodeFlags::IS_INLINE_ROOT.bits(), 0b001);
        assert_eq!(NodeFlags::IS_TABLE_ROOT.bits(), 0b010);
        assert_eq!(NodeFlags::IS_IN_DOCUMENT.bits(), 0b100);
    }
}

#[cfg(test)]
mod is_non_rendered_html_element_tests {
    //! The `<template>` path tests
    //! trigger `is_in_document()` first, leaving little direct coverage
    //! of the predicate itself. Test the DOM predicate directly with a builder
    //! and namespace mutation, including its namespace branch.

    use super::*;

    fn html_element(tag: &str) -> Node {
        Node::new_element(SmolStr::new(tag), taffy::Style::default(), None)
    }

    fn set_ns(n: &mut Node, ns: &str) {
        if let NodeData::Element(e) = &mut n.data {
            e.namespace = Some(SmolStr::new(ns));
        }
    }

    /// Nine elements from d9y.5 plus four added for complete §15.3.1 coverage
    /// in s8w (datalist / noembed / noframes / rp). The tag list matches
    /// the arms of `Node::is_non_rendered_html_element` one to one.
    const SKIP_SET_TAGS: &[&str] = &[
        // d9y.5 original:
        "head", "title", "meta", "link", "base", "noscript", "script", "style", "template",
        // s8w additions (complete §15.3.1 coverage):
        "datalist", "noembed", "noframes", "rp",
    ];

    #[test]
    fn predicate_true_for_html_default_namespace_skip_set() {
        for tag in SKIP_SET_TAGS {
            let n = html_element(tag);
            assert!(
                n.is_non_rendered_html_element(),
                "{tag} in HTML default namespace (None) must be non-rendered"
            );
        }
    }

    #[test]
    fn predicate_true_for_explicit_xhtml_namespace_skip_set() {
        for tag in SKIP_SET_TAGS {
            let mut n = html_element(tag);
            set_ns(&mut n, "http://www.w3.org/1999/xhtml");
            assert!(
                n.is_non_rendered_html_element(),
                "{tag} with explicit xhtml namespace must be non-rendered"
            );
        }
    }

    #[test]
    fn predicate_false_for_svg_namespace_same_named_elements() {
        // SVG <title>, <style>, and <script> are rendered / effective in SVG.
        // The predicate must filter only the HTML namespace; paint will handle
        // SVG rendering in a separate future pipeline.
        for tag in ["title", "style", "script"] {
            let mut n = html_element(tag);
            set_ns(&mut n, "http://www.w3.org/2000/svg");
            assert!(
                !n.is_non_rendered_html_element(),
                "SVG {tag} must NOT be filtered — SVG rendering owns these"
            );
        }
    }

    #[test]
    fn predicate_false_for_mathml_namespace_same_named_elements() {
        for tag in ["style", "script"] {
            let mut n = html_element(tag);
            set_ns(&mut n, "http://www.w3.org/1998/Math/MathML");
            assert!(
                !n.is_non_rendered_html_element(),
                "MathML {tag} must NOT be filtered"
            );
        }
    }

    #[test]
    fn predicate_false_for_normal_html_elements() {
        for tag in [
            "p", "div", "span", "h1", "a", "body", "html", "img", "table",
        ] {
            let n = html_element(tag);
            assert!(
                !n.is_non_rendered_html_element(),
                "{tag} is rendered content — predicate must return false"
            );
        }
    }

    #[test]
    fn predicate_false_for_non_element_nodes() {
        // Text / Document nodes are not Elements, so return false.
        let text = Node::new_text(SmolStr::new("hi"));
        assert!(!text.is_non_rendered_html_element());
        let doc = Node::new_document();
        assert!(!doc.is_non_rendered_html_element());
    }
}
