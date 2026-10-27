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
        ".missing".repeat(c)
    } else {
        (0..c).map(|i| format!(".c{i}")).collect::<String>()
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
    let (doc, tree, ids) = fixture(n, c, a, missing);
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
        "CLASS_INDEX n={n} c={c} a={a} loops={loops} missing={missing} ns={}",
        started.elapsed().as_nanos()
    );
}
