use super::*;

use raikiri_style::{PseudoElem, StyleNodeId, build_rule_tree, cascade};
use raikiri_traits::Symbol;
use taffy::Style;

#[test]
fn generated_ids_round_trip_and_stay_above_document_ids() {
    for element in [0, 1, 42, usize::MAX >> 3] {
        for pseudo in [PseudoElem::Before, PseudoElem::After] {
            let id = generated_node_id(element, pseudo);
            assert!(id > usize::MAX >> 3, "{element} {pseudo:?}");
            assert_eq!(generated_origin(id), Some((element, pseudo)));
        }
    }
    assert_eq!(generated_origin(usize::MAX >> 3), None);
}

#[test]
fn pseudo_counter_view_matches_directive_application_without_cloning_the_map() {
    let mut document = Document::new();
    let style = document.append_element(
        Some(document.root_index()),
        "style",
        Style::default(),
        None::<&str>,
    );
    document.append_text(
        style,
        "div::before { content: counters(existing, \".\"); counter-reset: existing 6 reset_only 4; counter-increment: existing 3 increment_only 2; counter-set: existing 11 set_only 9 }",
    );
    let element = document.append_element(
        Some(document.root_index()),
        "div",
        Style::default(),
        None::<&str>,
    );
    document.mark_in_document_flags();
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade succeeds");
    let pseudo = cascade
        .pseudo
        .get(&(StyleNodeId::new(element as u64), PseudoElem::Before))
        .expect("before pseudo is computed");

    let mut base = CounterSnapshot::default();
    base.insert(Symbol::new("existing"), vec![1, 2]);
    base.insert(Symbol::new("reset_only"), vec![7]);
    base.insert(Symbol::new("increment_only"), vec![8, 9]);
    let mut expected = base.clone();
    apply_counter_directives_to_snapshot(&mut expected, pseudo);

    let view = CounterSnapshotView::new(&base, Some(pseudo));
    for name in ["existing", "reset_only", "increment_only", "set_only"] {
        let actual: Vec<_> = view.values_for(name).iter().collect();
        assert_eq!(
            actual,
            expected
                .get(&Symbol::new(name))
                .cloned()
                .unwrap_or_default()
        );
    }
}
