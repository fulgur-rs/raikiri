use super::*;
use std::cell::Cell;

thread_local! {
    static DONOR_LOOKUPS: Cell<usize> = const { Cell::new(0) };
}

pub(in super::super) fn record_donor_lookup() {
    DONOR_LOOKUPS.set(DONOR_LOOKUPS.get() + 1);
}

#[test]
fn deep_inline_svg_paint_inheritance_visits_linearly_many_donors() {
    for depth in [100, 500, 2_000, 8_000] {
        let mut doc = TestDoc::new();
        let style = doc.push_element(0, "style", None);
        doc.push_text(style, "div { fill: red }");
        let mut parent = doc.push_element(0, "html", None);
        let mut roots = Vec::new();
        for _ in 0..depth {
            parent = doc.push_element(parent, "div", None);
            roots.push(doc.push_element_with_namespace(parent, "svg", SVG_NAMESPACE, &[]));
        }
        let tree = build_rule_tree(&doc);
        DONOR_LOOKUPS.set(0);
        let result = cascade(&doc, &tree).unwrap();
        let lookups = DONOR_LOOKUPS.get();
        assert_eq!(result.computed.len(), doc.nodes.len());
        for root in roots {
            assert_eq!(
                exported(
                    result.svg_style_properties(StyleNodeId(root as u64)),
                    PropertyKey::Fill
                ),
                Some((false, Some("rgba(255,0,0,1)".into())))
            );
        }
        eprintln!("painting donor work: depth={depth}, lookups={lookups}");
        assert!(lookups <= depth * PropertyKey::SVG_PRESENTATION.len());
    }
}

fn paint(key: PropertyKey, inherited: bool, expression: Option<&str>) -> SvgStyleProperty {
    SvgStyleProperty::new(key, inherited, expression.map(str::to_owned), None)
}

#[test]
fn cached_donors_match_the_legacy_scan_including_export_order_and_missing_expressions() {
    let ids = [
        StyleNodeId(7),
        StyleNodeId(2),
        StyleNodeId(5),
        StyleNodeId(1),
    ];
    let mut computed = vec![ComputedValues::initial(); 8];
    computed[7].font_size = ComputedLength(20.0);
    computed[2].font_size = ComputedLength(30.0);
    let exported = HashMap::from([
        (
            ids[0],
            vec![
                paint(PropertyKey::StrokeWidth, false, Some("2em")),
                paint(PropertyKey::Fill, false, Some("red")),
                paint(PropertyKey::Stroke, false, Some("green")),
            ],
        ),
        (
            ids[1],
            vec![
                paint(PropertyKey::StrokeDasharray, false, Some("1ex 2em 3%")),
                paint(PropertyKey::Fill, true, Some("inherit")),
                paint(PropertyKey::Stroke, false, None),
            ],
        ),
        (
            ids[2],
            vec![
                paint(PropertyKey::ClipRule, false, Some("evenodd")),
                paint(PropertyKey::Fill, false, Some("blue")),
            ],
        ),
    ]);
    for count in 0..=ids.len() {
        let ancestors = &ids[..count];
        let mut donors = PaintingDeclarers::default();
        for (depth, id) in ancestors.iter().enumerate() {
            donors = donors.declared_by(exported.get(id).map(Vec::as_slice).unwrap_or(&[]), depth);
        }
        for mask in 0..16 {
            let mut attributes = [false; PropertyKey::SVG_PRESENTATION.len()];
            attributes[0] = mask & 1 != 0;
            attributes[4] = mask & 2 != 0;
            let own = if mask & 4 != 0 {
                vec![paint(PropertyKey::Fill, false, Some("black"))]
            } else if mask & 8 != 0 {
                vec![paint(PropertyKey::Stroke, true, None)]
            } else {
                Vec::new()
            };
            for font_size in [16.0, 20.0, 30.0] {
                let mut expected = own.clone();
                legacy_complete(
                    &mut expected,
                    attributes,
                    ancestors,
                    &exported,
                    &computed,
                    font_size,
                );
                let mut actual = own.clone();
                inherit_ancestor_paint(
                    &mut actual,
                    attributes,
                    donors,
                    ancestors,
                    &exported,
                    &computed,
                    font_size,
                );
                assert_eq!(
                    actual, expected,
                    "count={count}, mask={mask}, font={font_size}"
                );
            }
        }
    }
}

#[test]
fn painting_environments_follow_branches_namespaces_and_shared_nodes() {
    let mut doc = TestDoc::new();
    let section = doc.push_element(
        0,
        "section",
        Some("--paint:red;fill:var(--paint);stroke:green;font-size:20px;stroke-width:1em"),
    );
    let inherited = doc.push_element(section, "div", Some("fill:inherit;stroke:unset"));
    doc.push_text(inherited, "text between elements");
    let outer = doc.push_element_with_namespace(inherited, "svg", SVG_NAMESPACE, &[]);
    doc.nodes[outer].inline_style = Some("font-size:10px".into());
    let outer_shared = doc.push_element_with_namespace(inherited, "svg", SVG_NAMESPACE, &[]);
    doc.nodes[outer_shared].inline_style = Some("font-size:10px".into());
    let mut inner_roots = Vec::new();
    for outer in [outer, outer_shared] {
        let foreign = doc.push_element_with_namespace(outer, "foreignObject", SVG_NAMESPACE, &[]);
        let attribute =
            doc.push_element_with_namespace(foreign, "g", SVG_NAMESPACE, &[("fill", "blue")]);
        let html = doc.push_element(attribute, "div", Some("font-size:30px"));
        inner_roots.push(doc.push_element_with_namespace(html, "svg", SVG_NAMESPACE, &[]));
    }
    let reset = doc.push_element(
        section,
        "div",
        Some("fill:initial;stroke-width:2ex;font-size:40px"),
    );
    let reset_svg = doc.push_element_with_namespace(reset, "svg", SVG_NAMESPACE, &[]);
    doc.nodes[reset_svg].inline_style = Some("font-size:10px".into());
    let sibling = doc.push_element(section, "div", None);
    let sibling_svg = doc.push_element_with_namespace(sibling, "svg", SVG_NAMESPACE, &[]);
    let detached = doc.push_element(section, "div", Some("fill:blue"));
    doc.nodes[detached].in_document = false;
    let detached_svg = doc.push_element_with_namespace(detached, "svg", SVG_NAMESPACE, &[]);
    let independent = doc.push_element_with_namespace(0, "svg", SVG_NAMESPACE, &[]);
    let tree = build_rule_tree(&doc);
    let (shared, shared_nodes) = walk_outputs(&doc, &tree, true);
    let (unshared, _) = walk_outputs(&doc, &tree, false);
    assert!(shared_nodes > 0);
    assert_eq!(shared.svg_properties, unshared.svg_properties);
    let properties = |id| {
        shared
            .svg_properties
            .get(&StyleNodeId(id as u64))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    };
    for id in [
        outer,
        outer_shared,
        inner_roots[0],
        inner_roots[1],
        sibling_svg,
    ] {
        assert_eq!(
            exported(properties(id), PropertyKey::Fill),
            Some((false, Some("rgba(255,0,0,1)".into())))
        );
        assert_eq!(
            exported(properties(id), PropertyKey::Stroke),
            Some((false, Some("rgba(0,128,0,1)".into())))
        );
    }
    assert_eq!(
        exported(properties(inner_roots[0]), PropertyKey::StrokeWidth),
        Some((false, Some("20px".into())))
    );
    assert_eq!(
        exported(properties(reset_svg), PropertyKey::Fill),
        Some((false, Some("black".into())))
    );
    assert_eq!(
        exported(properties(reset_svg), PropertyKey::StrokeWidth),
        Some((false, Some("40px".into())))
    );
    assert!(properties(detached_svg).is_empty());
    assert!(properties(independent).is_empty());
}

// The original full ancestor scan is retained only as an output-order oracle.
fn legacy_complete(
    properties: &mut Vec<SvgStyleProperty>,
    attribute_set: [bool; PropertyKey::SVG_PRESENTATION.len()],
    ancestors: &[StyleNodeId],
    exported: &HashMap<StyleNodeId, Vec<SvgStyleProperty>>,
    computed: &[ComputedValues],
    font_size_px: f32,
) {
    // A property the root sets itself, in a declaration or by an attribute,
    // is not the ancestors' to give.
    let mut pending = crate::property::PropertyKey::SVG_PRESENTATION.map(|(key, _, _)| {
        !properties
            .iter()
            .any(|property| property.property == key && !property.inherited)
    });
    for (pending, set) in pending.iter_mut().zip(attribute_set) {
        *pending &= !set;
    }
    for ancestor in ancestors.iter().rev() {
        let Some(listed) = exported.get(ancestor) else {
            continue;
        };
        for entry in listed {
            let Some(slot) = svg_presentation_slot(entry.property)
                .filter(|&slot| pending[slot] && !entry.inherited)
            else {
                continue;
            };
            pending[slot] = false;
            let in_px = computed
                .get(ancestor.0 as usize)
                .and_then(|c| declared_length_in_px(entry, c.font_size.0, font_size_px));
            let value = SvgStyleProperty {
                property: entry.property,
                inherited: false,
                expression: in_px.or_else(|| entry.expression.clone()),
                inherited_from: None,
            };
            match properties
                .iter_mut()
                .find(|property| property.property == entry.property)
            {
                Some(own) => *own = value,
                None => properties.push(value),
            }
        }
    }
}
