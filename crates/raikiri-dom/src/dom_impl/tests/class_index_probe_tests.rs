use crate::Document;
use raikiri_style::ruletree::{Origin, RuleTree};
use raikiri_style::{CssColor, cascade};

const RED: CssColor = CssColor {
    r: 255,
    g: 0,
    b: 0,
    a: 255,
};

pub(super) fn fixture(
    n: usize,
    c: usize,
    a: usize,
    missing: bool,
) -> (Document, RuleTree, Vec<usize>) {
    fixture_with_required(n, c, a, missing, c)
}

fn fixture_with_required(
    n: usize,
    c: usize,
    a: usize,
    missing: bool,
    required: usize,
) -> (Document, RuleTree, Vec<usize>) {
    let mut doc = Document::new();
    let class = (0..c)
        .map(|i| format!("c{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut ids = Vec::new();
    for _ in 0..n {
        let id = doc.append_element(Some(doc.root_index()), "p", None::<&str>);
        let mut attrs = (0..a)
            .map(|i| (format!("data-a{i}").into(), "x".into()))
            .collect::<Vec<_>>();
        attrs.push(("class".into(), class.as_str().into()));
        doc.set_element_attributes(id, attrs);
        ids.push(id);
    }
    let source = if missing {
        ".missing".repeat(required)
    } else {
        (0..required).map(|i| format!(".c{i}")).collect::<String>()
    };
    let mut tree = RuleTree::empty();
    // The universal subject key keeps the mismatch probe on the matching path.
    tree.add_stylesheet(&format!(":is({source}) {{color:red}}"), Origin::Author);
    (doc, tree, ids)
}

#[test]
#[ignore = "explicit release native class-index measurement"]
fn native_class_index_scaling_probe() {
    let env_usize = |name: &str, default| {
        std::env::var(name)
            .ok()
            .map(|v| v.parse::<usize>().unwrap())
            .unwrap_or(default)
    };
    let n = env_usize("CLASS_INDEX_N", 64);
    let c = env_usize("CLASS_INDEX_C", 32);
    let a = env_usize("CLASS_INDEX_A", 0);
    let loops = env_usize("CLASS_INDEX_LOOPS", 5);
    let missing = std::env::var("CLASS_INDEX_MISSING").as_deref() == Ok("1");
    let required = env_usize("CLASS_INDEX_REQUIRED", c);
    let (doc, tree, ids) = fixture_with_required(n, c, a, missing, required);
    let started = std::time::Instant::now();
    for _ in 0..loops {
        let output = cascade(std::hint::black_box(&doc), std::hint::black_box(&tree)).unwrap();
        assert!(
            ids.iter()
                .all(|id| (output.computed[*id].color == RED) != missing)
        );
        std::hint::black_box(output);
    }
    println!(
        "CLASS_INDEX n={n} c={c} required={required} a={a} loops={loops} missing={missing} ns={}",
        started.elapsed().as_nanos()
    );
}

#[test]
#[ignore = "explicit release native selector-list measurement"]
fn native_selector_list_scaling_probe() {
    let env_usize = |name: &str, default| {
        std::env::var(name)
            .ok()
            .map(|v| v.parse::<usize>().unwrap())
            .unwrap_or(default)
    };
    let n = env_usize("SELECTOR_INDEX_N", 64);
    let s = env_usize("SELECTOR_INDEX_S", 64);
    let loops = env_usize("SELECTOR_INDEX_LOOPS", 5);
    let grouped = std::env::var("SELECTOR_INDEX_SHAPE").as_deref() != Ok("separate");
    let mut doc = Document::new();
    let mut ids = Vec::new();
    for i in 0..n {
        let id = doc.append_element(Some(doc.root_index()), "p", None::<&str>);
        doc.set_element_attributes(id, vec![("class".into(), format!("c{}", i % s).into())]);
        ids.push(id);
    }
    let css = if grouped {
        format!(
            "{} {{color:red}}",
            (0..s)
                .map(|i| format!(".c{i}"))
                .collect::<Vec<_>>()
                .join(",")
        )
    } else {
        (0..s)
            .map(|i| format!(".c{i} {{color:red}}"))
            .collect::<String>()
    };
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(&css, Origin::Author);
    let started = std::time::Instant::now();
    for _ in 0..loops {
        let output = cascade(std::hint::black_box(&doc), std::hint::black_box(&tree)).unwrap();
        assert!(ids.iter().all(|id| output.computed[*id].color == RED));
        std::hint::black_box(output);
    }
    println!(
        "SELECTOR_LIST n={n} s={s} grouped={grouped} loops={loops} ns={}",
        started.elapsed().as_nanos()
    );
}
