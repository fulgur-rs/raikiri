use super::*;
use crate::layout::test_support::{page_box_800x600, with_ahem, with_bridged_session};
use raikiri_style::{build_rule_tree, cascade};

fn layout(
    tree: &mut Document,
    parent: usize,
    chain: &[usize],
    context: FragmentationContext,
    border_box: Size<f32>,
    origin: Point<f32>,
) -> Option<f32> {
    with_bridged_session(tree, |session| {
        super::layout(session, parent, chain, context, border_box, origin)
    })
}

fn fixture(depth: usize) -> (Document, usize, Vec<usize>) {
    fixture_with_margin(depth, "0")
}

fn fixture_with_margin(depth: usize, margin: &str) -> (Document, usize, Vec<usize>) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;font:20px/20px Ahem"),
    );
    let mc = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:100px;height:40px;column-count:2;column-gap:20px;orphans:1;widows:1"),
    );
    let mut chain = Vec::new();
    let mut parent = mc;
    for _ in 0..depth {
        parent = doc.append_element(
            Some(parent),
            "div",
            Some(format!("display:block;width:80px;margin:{margin}").as_str()),
        );
        chain.push(parent);
    }
    doc.append_text(parent, "A");
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).unwrap();
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
    (doc, mc, chain)
}

#[test]
fn cyclic_chains_are_rejected_without_returning_an_incomplete_path() {
    let (mut doc, mc, chain) = fixture(2);
    let leaf = *chain.last().unwrap();
    // The cycle closes after the bridge walked the tree.
    let cascade = crate::layout::test_support::cascaded(&doc);
    let mut session = crate::layout::context::LayoutSession::bridged(&mut doc, &cascade);
    session.nodes[leaf].ifc = None;
    session.nodes[leaf].children = vec![chain[0]];
    assert!(supported(&session, mc).is_none());
}

#[test]
fn taffy_calc_margins_resolve_against_the_fragmentainer_width() {
    let (mut doc, mc, chain) = fixture_with_margin(1, "0 0 0 calc(1px + 10%)");
    doc.fragment_tree.replace_finalized_fragments(Vec::new());
    let context = FragmentationContext {
        rtl: false,
        available_width: 100.0,
        available_height: Some(40.0),
        column_fill: raikiri_style::property::ColumnFillValue::Auto,
        column_width: 40.0,
        column_count: 2,
        column_gap: 20.0,
        column_index: 0,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 1,
    };
    assert_eq!(
        layout(
            &mut doc,
            mc,
            &chain,
            context,
            Size {
                width: 100.0,
                height: 40.0,
            },
            Point::ZERO,
        ),
        Some(40.0)
    );
    let fragment = doc
        .fragment_tree
        .fragments
        .iter()
        .find(|fragment| fragment.node_id == chain[0])
        .unwrap();
    assert_eq!(fragment.rect.x, 5.0);
    assert_eq!(fragment.rect.width, 80.0);
}

#[test]
fn column_clips_keep_overhanging_ruby_annotations() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;font:20px/20px Ahem"),
    );
    let mc = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:100px;height:40px;column-count:2;column-gap:20px;orphans:1;widows:1"),
    );
    let leaf = doc.append_element(Some(mc), "div", Some("display:block;width:80px;margin:0"));
    let ruby = doc.append_element(
        Some(leaf),
        "ruby",
        Some("display:ruby;ruby-position:inter-character"),
    );
    let base = doc.append_element(Some(ruby), "rb", Some("display:ruby-base"));
    doc.append_text(base, "AAAA");
    let annotation = doc.append_element(Some(ruby), "rt", Some("display:ruby-text"));
    doc.append_text(annotation, "XX");
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).unwrap();
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
    doc.fragment_tree.replace_finalized_fragments(Vec::new());
    let context = FragmentationContext {
        rtl: false,
        available_width: 100.0,
        available_height: Some(40.0),
        column_fill: raikiri_style::property::ColumnFillValue::Auto,
        column_width: 40.0,
        column_count: 2,
        column_gap: 20.0,
        column_index: 0,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 1,
    };
    layout(
        &mut doc,
        mc,
        &[leaf],
        context,
        Size {
            width: 100.0,
            height: 40.0,
        },
        Point::ZERO,
    )
    .unwrap();
    let clip = doc
        .fragment_tree
        .fragments
        .iter()
        .find(|fragment| fragment.node_id == leaf)
        .and_then(|fragment| fragment.fragmentainer_clip)
        .unwrap();
    // The inter-character annotation stands beside the end of the 80px
    // base, past the box, inside the ruby's inline box.
    assert_eq!((clip.x, clip.width), (0.0, 100.0), "{clip:?}");
}
