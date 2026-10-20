use super::*;

fn nonmatching_svg(rules: usize, elements: usize, styles: usize) -> String {
    let mut source = "<svg xmlns=\"http://www.w3.org/2000/svg\"><style>".to_owned();
    for rule in 0..rules {
        source.push_str(&format!(".missing-{rule} {{ fill:red }}"));
    }
    source.push_str("</style>");
    for _ in 1..styles {
        source.push_str("<style>/* empty */</style>");
    }
    for _ in 0..elements {
        source.push_str("<rect width=\"1\" height=\"1\"/>");
    }
    source.push_str("</svg>");
    source
}

#[test]
fn stylesheet_range_lookup_is_shared_across_selector_rules() {
    let mut observations = Vec::new();
    for (rules, elements, styles) in [(1, 16, 1), (16, 16, 1), (16, 128, 1), (16, 128, 32)] {
        let source = nonmatching_svg(rules, elements, styles);
        let mut times = Vec::new();
        let mut work = (0, 0);
        for _ in 0..7 {
            SELECTOR_FREEZE_STATS.with(|stats| stats.set((0, 0)));
            let start = std::time::Instant::now();
            let (frozen, changed) =
                freeze_svg_stylesheet_selectors(&source, &mut SelectorFreezeBudget::new()).unwrap();
            times.push(start.elapsed().as_micros());
            assert!(changed);
            assert!(!frozen.contains("data-raikiri-svg-selector"));
            work = SELECTOR_FREEZE_STATS.with(|stats| stats.get());
        }
        times.sort_unstable();
        eprintln!(
            "R={rules} E={elements} S={styles}: candidates={} range-comparisons={} median-us={}",
            work.0, work.1, times[3]
        );
        assert_eq!(work.0, rules * (elements + styles + 1));
        observations.push((work.1, 2 * (elements + styles + 1) + styles));
    }
    assert!(
        observations.iter().all(|&(actual, bound)| actual <= bound),
        "{observations:?}"
    );
}

#[test]
fn candidate_ranges_keep_style_boundaries_and_non_css_markup() {
    let source = r#"<svg><g id="before"/><style>* { fill:red }</style><g id="between"/>
        <style><g id="inside"/></style><g id="after"/>
        <style type="text/plain"><g id="non-css"/></style></svg>"#;
    let (frozen, _) =
        freeze_svg_stylesheet_selectors(source, &mut SelectorFreezeBudget::new()).unwrap();
    let xml = roxmltree::Document::parse(&frozen).unwrap();
    for id in ["before", "between", "after", "non-css"] {
        let node = xml
            .descendants()
            .find(|node| node.attribute("id") == Some(id))
            .unwrap();
        assert!(
            node.attributes()
                .any(|attribute| attribute.name().starts_with("data-raikiri-svg-selector")),
            "{id}"
        );
    }
    assert!(!frozen.contains("id=\"inside\""));
}

#[test]
fn candidate_preparation_charges_checks_and_reserved_bytes() {
    let source = "<svg><g/><style>* { fill:red }</style></svg>";
    let xml = roxmltree::Document::parse(source).unwrap();
    let root = xml.root_element();
    let capacity = root.descendants().len();
    let bytes = capacity * std::mem::size_of::<roxmltree::Node<'_, '_>>();
    for (checks, bytes) in [(capacity - 1, usize::MAX), (usize::MAX, bytes - 1)] {
        let mut budget = SelectorFreezeBudget {
            checks,
            bytes,
            matches: usize::MAX,
        };
        assert!(super::super::selector_freeze_candidates(root, &[], &mut budget).is_err());
    }
}
