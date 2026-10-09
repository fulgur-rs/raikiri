use super::*;

#[test]
fn frozen_property_names_are_sorted_and_still_supported() {
    assert!(PROPERTY_NAMES.windows(2).all(|pair| pair[0] < pair[1]));
    let supported = raikiri_style::property::supported_property_names();
    for name in PROPERTY_NAMES {
        assert!(
            supported.contains(name),
            "{name} is no longer supported; remove it from properties.rs"
        );
    }
}

#[test]
fn most_cases_have_a_designated_first_line_block() {
    let mut with_block = 0;
    for seed in 0..100 {
        let case = generate(seed);
        let Some(root) = case.first_line_root else {
            continue;
        };
        with_block += 1;
        let node = &case.doc.nodes[root];
        assert_eq!(node.tag, "p", "seed {seed}");
        assert!(node.in_document, "seed {seed}");
        assert!(
            node.attrs
                .iter()
                .any(|(name, value)| name == "id" && value == "fl")
        );
        assert!(
            case.sheets
                .iter()
                .any(|(css, _)| css.contains("#fl::first-line"))
        );
        assert!(describe(&case).contains(&format!("first-line root [{root}]")));
    }
    assert!(with_block > 60, "{with_block}");
}

#[test]
fn a_document_without_an_html_element_gets_no_first_line_block() {
    let mut doc = GenDoc::new(StyleQuirksMode::NoQuirks);
    assert_eq!(first_line_block(&mut Rng::new(0), &mut doc), None);
    let mut svg = GenNode::element("svg");
    svg.namespace = Some(SVG_NAMESPACE);
    doc.append(0, svg);
    assert_eq!(first_line_block(&mut Rng::new(0), &mut doc), None);
}

#[test]
fn a_seed_always_generates_the_same_case() {
    for seed in [0, 1, 7, 12_345] {
        assert_eq!(
            describe(&generate(seed)),
            describe(&generate(seed)),
            "seed {seed}"
        );
    }
}

#[test]
fn distinct_seeds_generate_distinct_cases() {
    let cases: Vec<String> = (0..20).map(|seed| describe(&generate(seed))).collect();
    for (i, a) in cases.iter().enumerate() {
        for b in &cases[i + 1..] {
            assert_ne!(a, b);
        }
    }
}

#[test]
fn generated_trees_are_consistent() {
    for seed in 0..200 {
        let case = generate(seed);
        let nodes = &case.doc.nodes;
        assert!(
            case.sheets
                .iter()
                .any(|(_, origin)| *origin == Origin::Author)
        );
        for (id, node) in nodes.iter().enumerate() {
            for &child in &node.children {
                assert_eq!(nodes[child].parent, Some(id), "seed {seed} node {child}");
                // A node outside the document never has a child inside it.
                assert!(
                    node.in_document || !nodes[child].in_document,
                    "seed {seed} node {child}"
                );
            }
        }
    }
}
