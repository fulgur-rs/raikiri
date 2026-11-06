use super::*;

#[test]
fn declaration_recovery_keeps_paint_and_geometry_from_the_start_of_a_rule() {
    let mut sheet = simplecss::StyleSheet::new();
    parse_more(
        &mut sheet,
        "rect{width;height:30px}circle{x:calc(min(1px,2px));fill:red !IMPORTANT}",
    );
    assert_eq!(sheet.rules.len(), 2);
    assert_eq!(
        sheet.rules[0].declarations,
        [simplecss::Declaration {
            name: "height",
            value: "30px",
            important: false
        }]
    );
    assert_eq!(
        sheet.rules[1].declarations,
        [
            simplecss::Declaration {
                name: "x",
                value: "calc(min(1px,2px))",
                important: false
            },
            simplecss::Declaration {
                name: "fill",
                value: "red",
                important: true
            }
        ]
    );
}

#[test]
fn strings_comments_and_nested_blocks_do_not_expose_rules() {
    let mut rules = Vec::new();
    visit_rules(
        "@media all{rect{fill:red}} rect{unknown:fn(\"};g{\",min(1,2));x:{a;b};fill:blue} circle{fill:green}",
        |rule| rules.push(rule),
    );
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0].selectors.trim(), "rect");
    assert!(rules[0].body.ends_with("fill:blue"));
    assert_eq!(rules[1].selectors.trim(), "circle");
}

#[test]
fn backend_recovery_preserves_the_unsupported_selector_boundary() {
    let source = "<svg><style>rect{width;fill:red} :is(rect),rect{fill:blue}</style><rect/></svg>";
    let recovered = backend_source(source).unwrap();
    assert!(recovered.contains("rect{fill:red;}"));
    assert!(!recovered.contains("fill:blue"));
}

#[test]
fn backend_expansion_stops_before_copying_shared_long_values() {
    let selectors = vec!["rect"; 128].join(",");
    let source = format!(
        "<svg><style>{selectors}{{broken;fill:red;unused:{}}} circle{{fill:blue}}</style><rect/></svg>",
        "a".repeat(70_000)
    );
    assert!(
        matches!(backend_source(&source), Err(crate::SvgError::InvalidDocument(message)) if message.contains("resource limit"))
    );
}
