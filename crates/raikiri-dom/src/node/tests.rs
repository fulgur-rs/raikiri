use super::*;
use crate::layout::test_support::with_ahem;

thread_local! {
    static TYPOGRAPHIC_PROBES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(super) fn record_typographic_probe() {
    TYPOGRAPHIC_PROBES.with(|count| count.set(count.get() + 1));
}

#[test]
fn retained_typographic_lookup_has_linear_work_across_many_inline_owners() {
    use crate::layout::ifc::test_support::{ahem_fonts, sheet_fixture};
    let mut fixture = sheet_fixture(
        "div::first-letter{color:red}",
        "width:10000px",
        |doc, root| {
            for _ in 0..128 {
                let span = doc.append_element(
                    Some(root),
                    "span",
                    taffy::Style::default(),
                    Some("display:inline"),
                );
                doc.append_text(span, "“");
            }
            doc.append_text(root, "A");
            for _ in 0..32 {
                let span = doc.append_element(
                    Some(root),
                    "span",
                    taffy::Style::default(),
                    Some("display:inline"),
                );
                doc.append_text(span, "Y");
            }
        },
    );
    fixture.doc.set_font_collection(ahem_fonts());
    crate::layout::layout_single_page(
        &mut fixture.doc,
        &fixture.cascade,
        crate::layout::test_support::page_box_800x600(),
    )
    .unwrap();
    let node = fixture.doc.get_node(fixture.root).unwrap();
    assert_eq!(node.ifc.as_ref().unwrap().letter_styles.len(), 129);
    let pieces = node.ifc_inline_boxes().unwrap();
    TYPOGRAPHIC_PROBES.with(|count| count.set(0));
    let mut retained = 0;
    for piece in &pieces {
        let result =
            node.ifc_typographic_fragment(piece.node, piece.source_container, piece.source_owner);
        if let Some((style, owner)) = result {
            retained += 1;
            assert_eq!(style.color.r, 255);
            assert_eq!(Some(owner), piece.source_owner);
        }
    }
    assert_eq!(retained, 129);
    let work = TYPOGRAPHIC_PROBES.with(std::cell::Cell::get);
    assert!(work > 0);
    assert!(
        work <= pieces.len() * 2,
        "{work} candidate probes for {} pieces",
        pieces.len()
    );
}

#[cfg(test)]
mod ifc_geometry_tests {
    use super::*;
    use crate::layout::layout_single_page;
    use crate::layout::test_support::{ahem_paragraph, ifc_ahem_fonts, page_box_800x600};

    #[test]
    fn a_laid_out_ifc_root_exposes_its_writing_mode_and_physical_content_size() {
        let css = "box-sizing:border-box;width:100px;height:40px;writing-mode:vertical-rl;padding:5px 4px 3px 6px;border:1px solid";
        let (mut doc, cascade, root) = ahem_paragraph("aa", css);
        doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
        layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");

        assert_eq!(
            doc.nodes[root].ifc_writing_mode(),
            Some(shodo::geometry::WritingMode::VerticalRl)
        );
        assert_eq!(
            doc.nodes[root].ifc_physical_content_size(),
            Some(shodo::geometry::PhysicalSize {
                width: 88.0,
                height: 30.0,
            })
        );
        assert_eq!(doc.nodes[root].ifc_size(), Some((30.0, 10.0)));
    }

    #[test]
    fn vertical_inline_size_constrains_lines_like_height() {
        let layout = |size_css: &str| {
            let css = format!("writing-mode:vertical-rl;{size_css}");
            let (mut doc, cascade, root) = ahem_paragraph("aaaa bbbb cccc", &css);
            doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
            layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
            let lines = doc.nodes[root].ifc_lines().expect("lines").len();
            (
                lines,
                doc.nodes[root].ifc_physical_content_size(),
                crate::layout::test_support::absolute_rect(&doc, root),
            )
        };

        let logical = layout("inline-size:40px");
        assert_eq!(logical, layout("height:40px"));
        // A `ch` inline size is measured onto the same physical axis.
        assert_eq!(layout("inline-size:4ch"), logical);
        assert_eq!(logical.0, 3, "40px of inline extent fits one word per line");
        assert_eq!(
            logical.1.map(|size| size.height),
            Some(40.0),
            "inline-size is the physical height in vertical-rl"
        );
    }

    #[test]
    fn ifc_inline_boxes_are_reported_in_vertical_physical_coordinates() {
        let css = "box-sizing:border-box;width:100px;height:40px;writing-mode:vertical-rl";
        let (mut doc, _, root) = ahem_paragraph("", css);
        let inline = doc.append_element(
            Some(root),
            "span",
            taffy::Style::default(),
            Some("display:inline;padding-left:3px;padding-right:3px"),
        );
        doc.append_text(inline, "bb");
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
        doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
        layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");

        let pieces = doc.nodes[root].ifc_inline_boxes().expect("pieces");
        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0].node, inline);
        assert_eq!(
            (pieces[0].border_box.x, pieces[0].border_box.y),
            (90.0, 0.0)
        );
        assert_eq!(
            (pieces[0].border_box.width, pieces[0].border_box.height),
            (16.0, 20.0)
        );
    }
}

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
    }

    #[test]
    fn node_new_document_fragment_kind_and_default_flag_state() {
        let n = Node::new_document_fragment();
        assert_eq!(n.kind(), NodeKind::DocumentFragment);
        // The Fragment root starts detached when used, so it becomes false after
        // marking; the constructor itself defaults to true.
        assert!(n.is_in_document());
        assert_eq!(n.tag_name(), None);
    }

    #[test]
    fn node_flags_bit_values_match_blitz_raw() {
        // Regression check: blitz `NodeFlags` must match the raw bit values
        // defined in blitz-dom/src/node/node.rs:50-58:
        //   IS_TABLE_ROOT = 0b010, IS_IN_DOCUMENT = 0b100 (bit 0, blitz's
        //   IS_INLINE_ROOT, is unused here)
        //
        // This makes future blitz-compat conversion a trivial
        // `NodeFlags::from_bits(x)` cast. Add future bits at the same positions
        // as blitz.
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

#[cfg(test)]
mod attribute_ns_tests {
    use super::*;

    const XLINK_NS: &str = "http://www.w3.org/1999/xlink";

    fn element_with_namespaced_attr() -> Node {
        let mut node = Node::new_element(SmolStr::new("use"), taffy::Style::default(), None);
        if let NodeData::Element(element) = &mut node.data {
            element.attributes.push(Attr {
                namespace: Some(SmolStr::new(XLINK_NS)),
                prefix: Some(SmolStr::new("xlink")),
                local: SmolStr::new("href"),
                value: SmolStr::new("#shape"),
            });
            element.attributes.push(Attr {
                namespace: None,
                prefix: None,
                local: SmolStr::new("href"),
                value: SmolStr::new("#plain"),
            });
        }
        node
    }

    #[test]
    fn attribute_ns_finds_namespaced_value_and_ignores_prefix() {
        let node = element_with_namespaced_attr();
        assert_eq!(node.attribute_ns(XLINK_NS, "href"), Some("#shape"));
        assert_eq!(node.attribute("href"), Some("#plain"));
    }

    #[test]
    fn attribute_ns_is_exact_and_returns_none_for_mismatch() {
        let node = element_with_namespaced_attr();
        assert_eq!(node.attribute_ns(XLINK_NS, "HREF"), None);
        assert_eq!(
            node.attribute_ns("http://www.w3.org/2000/svg", "href"),
            None
        );
        assert_eq!(node.attribute_ns(XLINK_NS, "title"), None);
    }

    #[test]
    fn attribute_ns_returns_none_for_non_elements() {
        let text = Node::new_text(SmolStr::new("hi"));
        assert_eq!(text.attribute_ns(XLINK_NS, "href"), None);
        let doc = Node::new_document();
        assert_eq!(doc.attribute_ns(XLINK_NS, "href"), None);
    }
}

#[test]
fn an_ifc_root_exposes_no_layout_children() {
    use crate::Document;
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", taffy::Style::default(), None::<&str>);
    let root = doc.append_element(Some(html), "div", taffy::Style::default(), None::<&str>);
    doc.append_text(root, "aa");
    assert_eq!(doc.nodes[root].layout_children().len(), 1);
    doc.nodes[root].flags.insert(NodeFlags::IS_IFC_ROOT);
    assert!(doc.nodes[root].layout_children().is_empty());
    // The DOM children are untouched: only the taffy view changes.
    assert_eq!(doc.nodes[root].children.len(), 1);
}

#[test]
fn a_laid_out_ifc_root_exposes_its_lines() {
    use crate::layout::layout_single_page;
    use crate::layout::test_support::{ahem_paragraph, ifc_ahem_fonts, page_box_800x600};
    let (mut doc, cascade, root) = ahem_paragraph("aaaa bbbb cccc", "width:50px");
    // Before a layout the block holds no lines.
    assert!(doc.nodes[root].ifc_lines().is_none());
    // After it: three 10px lines.
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    assert!(doc.nodes[root].is_ifc_root());
    assert_eq!(doc.nodes[root].ifc_lines().map(<[_]>::len), Some(3));
}

#[test]
fn canvas_bitmap_errors_have_descriptive_messages() {
    let cases = [
        (
            CanvasBitmapError::DimensionsTooLarge,
            "canvas bitmap dimensions exceed the per-canvas limit",
        ),
        (
            CanvasBitmapError::AllocationFailed,
            "canvas bitmap allocation failed",
        ),
        (
            CanvasBitmapError::DocumentLimitExceeded,
            "canvas bitmap exceeds the document memory limit",
        ),
        (
            CanvasBitmapError::InvalidRgbaLength,
            "canvas bitmap has an invalid RGBA buffer length",
        ),
        (
            CanvasBitmapError::SizeMismatch,
            "canvas bitmap dimensions do not match the canvas element",
        ),
        (CanvasBitmapError::NotCanvas, "node is not a canvas element"),
    ];

    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
    }
}

#[cfg(test)]
mod min_content_tests {
    use super::*;
    use crate::layout::layout_single_page;
    use crate::layout::test_support::{absolute_rect, ifc_ahem_fonts, page_box_800x600};
    use taffy::Style;

    /// A border box as `(x, y, width, height)`.
    type Rect = (f32, f32, f32, f32);

    /// The border box of `wrapper` and of its float ancestor, for a
    /// `width:min-content` wrapper around a block of Ahem text.
    fn min_content_rects(float: bool) -> (Rect, Rect) {
        let mut doc = crate::Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
        let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
        let outer = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some(if float {
                "display:block;float:left;font-family:Ahem;font-size:10px;line-height:10px"
            } else {
                "display:block;font-family:Ahem;font-size:10px;line-height:10px"
            }),
        );
        let wrapper = doc.append_element(
            Some(outer),
            "div",
            Style::default(),
            Some("display:block;width:min-content"),
        );
        let inner = doc.append_element(
            Some(wrapper),
            "div",
            Style::default(),
            Some("display:block"),
        );
        let text = doc.append_element(Some(inner), "div", Style::default(), Some("display:block"));
        doc.append_text(text, "aaaa bbbb cccc");
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
        doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
        layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
        (absolute_rect(&doc, wrapper), absolute_rect(&doc, outer))
    }

    #[test]
    fn a_min_content_wrapper_is_as_wide_as_its_longest_word() {
        let (wrapper, _) = min_content_rects(false);
        assert_eq!(wrapper.2, 40.0);
    }

    #[test]
    fn a_float_around_a_min_content_wrapper_shrinks_to_it() {
        let (wrapper, float) = min_content_rects(true);
        assert_eq!(wrapper.2, 40.0);
        assert_eq!(float.2, 40.0);
    }
}
