use crate::ruletree::SupportsContext;
use cssparser::ToCss;

fn nested_selector(depth: usize) -> String {
    format!("{}div{}", ":is(".repeat(depth), ")".repeat(depth))
}

// Execute adversarial parsing on the normal unit-test thread in a separate
// process: an upstream stack overflow must fail this test rather than abort
// every test in the workspace.
fn check_entry_point(entry: &str) {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "tests::selector_depth_tests::adversarial_selector_child",
            "--nocapture",
        ])
        .env("RAIKIRI_SELECTOR_DEPTH_ENTRY", entry)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{entry}: status={}\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn root_selector_depth_is_bounded() {
    check_entry_point("root");
}

#[test]
fn query_selector_depth_is_bounded() {
    check_entry_point("query");
}

#[test]
fn supports_selector_depth_is_bounded() {
    check_entry_point("supports");
}

#[test]
fn adversarial_selector_child() {
    let Ok(entry) = std::env::var("RAIKIRI_SELECTOR_DEPTH_ENTRY") else {
        return;
    };
    for depth in [33, 127, 132, 4000] {
        let source = nested_selector(depth);
        match entry.as_str() {
            "root" => {
                let mut tree = crate::RuleTree::empty();
                tree.add_stylesheet(
                    &format!("{source} {{color:red}} p {{color:blue}}"),
                    crate::Origin::Author,
                );
                assert_eq!(
                    tree.style_rules().len(),
                    1,
                    "reject deep rule and recover sibling"
                );
            }
            "query" => {
                assert!(crate::parse_selector_list(&source).is_err());
                assert!(crate::SelectorQuery::parse(&source).is_err());
            }
            "supports" => assert!(!crate::supports::supports_condition(
                &format!("selector({source})"),
                &SupportsContext::new("", &[])
            )),
            _ => panic!("unknown entry point"),
        }
    }
}

#[test]
fn ordinary_functional_selectors_keep_matching_and_specificity() {
    let source = nested_selector(32);
    let selectors = crate::parse_selector_list(&source).unwrap();
    assert_eq!(selectors.slice()[0].specificity(), 1);
    let mut doc = crate::test_dom::TestDoc::new();
    let div = doc.push_element(0, "div", None);
    assert!(crate::SelectorQuery::parse(&source).unwrap().matches(
        &doc,
        crate::StyleNodeId::new(div as u64),
        &[]
    ));
    assert!(crate::supports::supports_condition(
        &format!("selector({source})"),
        &SupportsContext::new("", &[])
    ));
    let mut tree = crate::RuleTree::empty();
    tree.add_stylesheet(&format!("{source} {{color:red}}"), crate::Origin::Author);
    assert_eq!(tree.style_rules().len(), 1);
}

#[test]
fn grouped_rules_recover_after_an_overdeep_selector() {
    let deep = nested_selector(4000);
    for group in ["@layer safe", "@supports (color: red)"] {
        let mut tree = crate::RuleTree::empty();
        tree.add_stylesheet(
            &format!("{group} {{{deep} {{color:red}} p {{color:blue}}}}"),
            crate::Origin::Author,
        );
        assert_eq!(tree.style_rules().len(), 1);
        assert_eq!(tree.style_rules()[0].selectors.to_css_string(), "p");
    }
}

#[test]
fn quoted_selector_attribute_delimiters_do_not_consume_depth_budget() {
    let source = format!(r#"[title="{}"]"#, "(".repeat(4000));
    assert!(crate::parse_selector_list(&source).is_ok());
    assert!(crate::SelectorQuery::parse(&source).is_ok());
    assert!(crate::supports::supports_condition(
        &format!("selector({source})"),
        &SupportsContext::new("", &[])
    ));
}
