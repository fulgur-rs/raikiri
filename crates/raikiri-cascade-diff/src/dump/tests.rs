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
        "div { display: list-item } div::first-line { color: red } \
         div::first-letter { font-size: 2em } div::before { content: \"B\" } \
         div::after { content: \"A\" } div::marker { color: green } \
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
        format!("first_letter[{div}] at [{div}, Some(After)]: ComputedValues"),
        format!("first_letter[{div}] at [{div}, Some(Marker)]: ComputedValues"),
    ] {
        assert!(out.contains(&line), "missing {line}:\n{out}");
    }
    // `span` has no `::before`, so it is only resolved as an ordinary parent.
    assert!(
        !out.contains(&format!("at [{span}, Some(Before)]")),
        "{out}"
    );
    assert!(!out.contains(CUSTOM_PROPERTY_ENVIRONMENT), "{out}");
}

#[test]
fn derived_styles_print_custom_properties_by_value() {
    let mut doc = GenDoc::new(StyleQuirksMode::NoQuirks);
    let p = doc.append(0, GenNode::element("p"));
    doc.append(p, GenNode::text("text"));
    let out = dump_of(
        &doc,
        "p { --a: 1 } p::first-letter { color: red }",
        &["--a", "--b"],
    );
    assert!(
        out.contains(&format!("first_letter[{p}] custom: --a=\"1\"\n")),
        "{out}"
    );
    assert!(!out.contains(CUSTOM_PROPERTY_ENVIRONMENT), "{out}");
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
         @page :first { margin: 1px } @page named { margin: 2px } \
         @page named:first { margin: 3px }",
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
    for query in ["first", "left", "named", "named first", "blank"] {
        assert!(out.contains(&format!("page[{query}]: ")), "{out}");
    }
    // The page results hold the root's custom-property environment, which
    // is not printed either.
    assert!(!out.contains(CUSTOM_PROPERTY_ENVIRONMENT), "{out}");
}

#[test]
fn page_margin_boxes_are_printed_as_layout_resolves_them() {
    let mut doc = GenDoc::new(StyleQuirksMode::NoQuirks);
    doc.append(0, GenNode::element("html"));
    // A page-local custom property used only in a margin box shows up in
    // the margin box's resolved declarations.
    let dump = |ink: &str| {
        dump_of(
            &doc,
            &format!("@page {{ --ink: {ink}; @top-center {{ color: var(--ink) }} }}"),
            &[],
        )
    };
    let red = dump("red");
    assert!(red.contains("page margin[TopCenter]: Some("), "{red}");
    assert!(
        red.contains("page[first] margin[TopCenter]: Some("),
        "{red}"
    );
    assert_ne!(red, dump("blue"));
    assert!(!red.contains(CUSTOM_PROPERTY_ENVIRONMENT), "{red}");
}

#[test]
fn a_panicking_section_keeps_its_output_and_the_rest_of_the_dump() {
    let mut out = String::from("before\n");
    section(&mut out, "probe", |out| {
        out.push_str("partial\n");
        panic!("boom");
    });
    section(&mut out, "next", |out| out.push_str("after\n"));
    assert_eq!(out, "before\npartial\nPANIC in probe: boom\nafter\n");
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
    first_line_styles(&mut out, None, &[]);
    assert_eq!(out, "first_line: None\n");
    let mut computed = vec![None, Some(ComputedValues::initial()), None];
    computed[2] = Some(ComputedValues::initial());
    let styles = FirstLineStyles {
        root: StyleNodeId::new(1),
        computed,
    };
    let mut out = String::new();
    first_line_styles(&mut out, Some(&styles), &["--a"]);
    assert!(
        out.starts_with("first_line.root: 1\nfirst_line[1]: ComputedValues\n"),
        "{out}"
    );
    assert!(out.contains("first_line[2]: ComputedValues\n"), "{out}");
    assert!(!out.contains("first_line[0]"), "{out}");
    assert!(!out.contains(CUSTOM_PROPERTY_ENVIRONMENT), "{out}");
}

#[test]
fn custom_property_names_are_scanned_from_the_source() {
    assert_eq!(
        custom_property_names(["a { --b: 1; --a-b_c: var(--b) } <!-- c --> --é- --"]),
        vec!["--a-b_c", "--b", "--é-"]
    );
    // An escaped name is found by its value, wherever the escapes are, in
    // any of the sources: a hex escape takes one whitespace after it, and a
    // CRLF counts as one.
    assert_eq!(
        custom_property_names([
            "--\\66 oo: 1",
            "p { \\2d\\2d bar: 2 }",
            "--\\62\r\naz: 3; --\\-q: 4"
        ]),
        vec!["---q", "--bar", "--baz", "--foo"]
    );
    // However deeply it is nested.
    let deep = format!("{}--\\64 eep: 1{}", "{".repeat(200), "}".repeat(200));
    assert_eq!(custom_property_names([deep.as_str()]), vec!["--deep"]);
    // A NUL, a surrogate or a code point past Unicode decodes to U+FFFD, a
    // backslash before a newline escapes nothing, and a hex escape stops
    // after six digits.
    assert_eq!(
        custom_property_names(["--a\\0 ; --b\\d800 ; --c\\110000 ; --d\\\n ; --e\\0000411"]),
        vec!["--a\u{fffd}", "--b\u{fffd}", "--c\u{fffd}", "--d", "--eA1"]
    );
    // So does a NUL in the source itself, which CSS reads as U+FFFD, escaped
    // or not.
    assert_eq!(
        custom_property_names(["--f\0g: 1; --h\\\0i: 2"]),
        vec!["--f\u{fffd}g", "--h\u{fffd}i"]
    );
    // A leading byte order mark is not part of the first name.
    assert_eq!(custom_property_names(["\u{feff}--j: 1"]), vec!["--j"]);
}

#[test]
fn fnv1a_matches_the_reference_values() {
    assert_eq!(fnv1a(""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(fnv1a("a"), 0xaf63_dc4c_8601_ec8c);
}
