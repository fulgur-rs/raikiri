use super::*;
use crate::ruletree::{Origin, RuleTree};
use std::fmt::Write as _;
use std::time::Instant;

#[test]
#[ignore = "manual dense custom-property cycle scaling probe"]
fn dense_custom_property_cycle_probe() {
    for count in [64, 128, 256, 512] {
        let mut value = String::new();
        for target in 0..count {
            write!(value, "var(--v{target}) ").unwrap();
        }
        let mut local = HashMap::new();
        let mut css = String::from("div {");
        for source in 0..count {
            local.insert(format!("--v{source}").into(), value.clone().into());
            write!(css, "--v{source}: {value};").unwrap();
        }
        css.push('}');
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(&css, Origin::Author);
        assert_eq!(tree.style_rules()[0].declarations().len(), count);
        let mut document = TestDoc::new();
        let element = document.push_element(0, "div", None);
        let mut cycles = Vec::new();
        let mut cascade_ms = Vec::new();
        for _ in 0..5 {
            let start = Instant::now();
            let members = find_cycle_members(&local);
            cycles.push(start.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(members.len(), count);
            let start = Instant::now();
            let result = cascade(&document, &tree).unwrap();
            cascade_ms.push(start.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(
                result.computed[element].resolved_custom_property("--v0"),
                None
            );
            std::hint::black_box(result);
        }
        cycles.sort_by(f64::total_cmp);
        cascade_ms.sort_by(f64::total_cmp);
        let graph: Vec<Vec<usize>> = (0..count).map(|_| (0..count).collect()).collect();
        let mut work = CycleWork::default();
        assert!(
            cycle_members_in_graph(&graph, &mut work)
                .into_iter()
                .all(|cyclic| cyclic)
        );
        println!(
            "vertices={count} edges={} cycle_ms={:.4} cascade_ms={:.4} graph_steps={}",
            count * count,
            cycles[2],
            cascade_ms[2],
            work.steps
        );
    }
}
