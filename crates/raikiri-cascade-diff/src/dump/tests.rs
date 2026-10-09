use super::*;
use crate::doc::{GenDoc, GenNode, SVG_NAMESPACE};
use raikiri_style::{Origin, StyleQuirksMode};

/// The dump of `doc` cascaded for print with one author stylesheet.
fn dump_of(doc: &GenDoc, css: &str, names: &[&str]) -> String {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(css, Origin::Author);
    let media = MediaContext::print();
    let inputs = Inputs {
        dom: doc,
        tree: &tree,
        media: &media,
        custom_names: names,
    };
    let result = raikiri_style::cascade_with_media_context(doc, &tree, &media).expect("cascade");
    let mut out = String::new();
    cascade_result(&mut out, &inputs, &result);
    out
}

#[test]
fn first_letter_styles_are_resolved_through_enclosing_first_lines() {
    let mut doc = GenDoc::new(StyleQuirksMode::NoQuirks);
    let div = doc.append(0, GenNode::element("div"));
    let span = doc.append(div, GenNode::element("span"));
    doc.append(span, GenNode::text("text"));
    let out = dump_of(
        &doc,
        "div { display: block } div::first-line { color: red } \
         div::first-letter { font-size: 2em } div::before { content: \"B\" } \
         span { color: blue }",
        &[],
    );
    assert!(out.contains("has_first_letter_styles: true"), "{out}");
    for line in [
        format!("first_letter[{div}]: ComputedValues"),
        format!("first_letter[{div}] at [{div}, None] parent: ComputedValues"),
        format!("first_letter[{div}] at [{div}, None]: ComputedValues"),
        format!("first_letter[{div}] at [{span}, None]: ComputedValues"),
        format!("first_letter[{div}] at [{div}, Some(Before)]: ComputedValues"),
    ] {
        assert!(out.contains(&line), "missing {line}:\n{out}");
    }
    // `span` has no `::before`, so it is only resolved as an ordinary parent.
    assert!(
        !out.contains(&format!("at [{span}, Some(Before)]")),
        "{out}"
    );
}

#[test]
fn first_letter_styles_without_a_first_line_are_resolved_over_their_origin() {
    let mut doc = GenDoc::new(StyleQuirksMode::NoQuirks);
    let p = doc.append(0, GenNode::element("p"));
    doc.append(p, GenNode::text("text"));
    let out = dump_of(&doc, "p::first-letter { color: red }", &[]);
    assert!(
        out.contains(&format!("first_letter[{p}]: ComputedValues")),
        "{out}"
    );
    assert!(!out.contains(" parent: "), "{out}");
    let out = dump_of(&doc, "p { color: red }", &[]);
    assert!(out.contains("has_first_letter_styles: false"), "{out}");
    assert!(!out.contains("first_letter["), "{out}");
}

#[test]
fn outputs_reached_only_through_methods_are_printed() {
    let mut doc = GenDoc::new(StyleQuirksMode::NoQuirks);
    let html = doc.append(0, GenNode::element("html"));
    let mut svg = GenNode::element("svg");
    svg.namespace = Some(SVG_NAMESPACE);
    let svg = doc.append(html, svg);
    let p = doc.append(html, GenNode::element("p"));
    let out = dump_of(
        &doc,
        "html { --v0: 3px } svg { color: inherit; opacity: 0.5 } \
         ::highlight(hl) { background-color: currentcolor } \
         @page :first { margin: 1px } @page named { margin: 2px }",
        &["--v0", "--v9"],
    );
    assert!(out.contains(&format!("svg[{svg}]: ")), "{out}");
    assert!(
        out.contains(&format!("custom[{p}]: --v0=\"3px\"\n")),
        "{out}"
    );
    assert!(!out.contains("--v9="), "{out}");
    let foreground = "CssColor { r: 10, g: 20, b: 30, a: 128 }";
    assert!(
        out.contains(&format!(
            "highlight[\"hl\", {foreground}]: Some({foreground})"
        )),
        "{out}"
    );
    for query in ["first", "left", "named", "blank"] {
        assert!(out.contains(&format!("page[{query}]: ")), "{out}");
    }
}

#[test]
fn custom_properties_of_pseudo_elements_are_printed() {
    let mut doc = GenDoc::new(StyleQuirksMode::NoQuirks);
    let p = doc.append(0, GenNode::element("p"));
    let out = dump_of(&doc, "p::before { --v1: x; content: \"b\" }", &["--v1"]);
    assert!(
        out.contains(&format!("custom[{p}, Before]: --v1=\"x\"")),
        "{out}"
    );
}

#[test]
fn the_first_line_styles_of_a_block_are_printed_per_node() {
    let mut out = String::new();
    first_line_styles(&mut out, None);
    assert_eq!(out, "first_line: None\n");
    let mut computed = vec![None, Some(ComputedValues::initial()), None];
    computed[2] = Some(ComputedValues::initial());
    let styles = FirstLineStyles {
        root: StyleNodeId::new(1),
        computed,
    };
    let mut out = String::new();
    first_line_styles(&mut out, Some(&styles));
    assert!(
        out.starts_with("first_line.root: 1\nfirst_line[1]: ComputedValues\n"),
        "{out}"
    );
    assert!(out.contains("first_line[2]: ComputedValues\n"), "{out}");
    assert!(!out.contains("first_line[0]"), "{out}");
}

#[test]
fn custom_property_names_are_scanned_from_the_source() {
    assert_eq!(
        custom_property_names("a { --b: 1; --a-b_c: var(--b) } <!-- c --> --é- --"),
        vec!["--a-b_c", "--b", "--é-"]
    );
}

#[test]
fn fnv1a_matches_the_reference_values() {
    assert_eq!(fnv1a(""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(fnv1a("a"), 0xaf63_dc4c_8601_ec8c);
}
