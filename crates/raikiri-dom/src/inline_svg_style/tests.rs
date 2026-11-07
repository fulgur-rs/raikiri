use super::*;
use crate::layout::ifc::test_support::sheet_fixture;
use crate::layout::test_support::INLINE_SVG_PARENT_STEPS;
use raikiri_style::cascade::SvgStyleProperty;
use raikiri_style::property::PropertyKey;
use smol_str::SmolStr;

#[test]
fn descendant_declarations_ignore_properties_outside_the_svg_contract() {
    for inherited in [false, true] {
        let properties = [SvgStyleProperty::new(
            PropertyKey::Width,
            inherited,
            None,
            None,
        )];
        let mut budget = 1024;
        assert_eq!(
            node_declarations(
                &ComputedValues::initial(),
                &properties,
                &[],
                false,
                &mut budget
            )
            .unwrap(),
            ""
        );
    }
}

#[test]
fn descendant_declaration_budget_rejects_before_serializing() {
    let properties = [SvgStyleProperty::new(
        PropertyKey::Opacity,
        false,
        None,
        None,
    )];
    let mut remaining = 128;
    assert!(
        node_declarations(
            &ComputedValues::initial(),
            &properties,
            &[],
            false,
            &mut remaining
        )
        .is_err()
    );
}

#[test]
fn descendant_declaration_budget_counts_the_text_of_a_painting_property() {
    let text = format!("url(\"#{}\")", "a".repeat(1000));
    let properties = [SvgStyleProperty::new(
        PropertyKey::Fill,
        false,
        Some(text.clone()),
        None,
    )];
    // The declaration's own 128 bytes, then the text's 128 and its two copies.
    let needed = 128 + 128 + 2 * text.len();
    let mut remaining = needed - 1;
    assert!(
        node_declarations(
            &ComputedValues::initial(),
            &properties,
            &[],
            false,
            &mut remaining
        )
        .is_err()
    );
    let mut remaining = needed;
    let css = node_declarations(
        &ComputedValues::initial(),
        &properties,
        &[],
        false,
        &mut remaining,
    )
    .unwrap();
    assert_eq!(css, format!("fill:{text}!important;"));
    assert_eq!(remaining, 0);
}

#[test]
fn descendant_declarations_keep_relative_expressions_and_literal_inherit() {
    let properties = [
        SvgStyleProperty::new(PropertyKey::FontSize, false, Some("2em".into()), None),
        SvgStyleProperty::new(PropertyKey::Color, true, None, None),
    ];
    let mut remaining = 1024;
    assert_eq!(
        node_declarations(
            &ComputedValues::initial(),
            &properties,
            &[],
            false,
            &mut remaining
        )
        .unwrap(),
        "font-size:2em!important;color:inherit!important;"
    );
}

#[test]
fn descendant_declarations_serialize_all_supported_computed_properties() {
    let keys = [
        PropertyKey::Color,
        PropertyKey::Display,
        PropertyKey::Opacity,
        PropertyKey::Visibility,
        PropertyKey::FontSize,
        PropertyKey::FontFamily,
        PropertyKey::FontStyle,
        PropertyKey::FontWeight,
    ];
    let properties: Vec<_> = keys
        .iter()
        .map(|&property| SvgStyleProperty::new(property, false, None, None))
        .collect();
    let mut budget = 4096;
    let css = node_declarations(
        &ComputedValues::initial(),
        &properties,
        &[],
        false,
        &mut budget,
    )
    .unwrap();
    for declaration in [
        "color:rgba(0,0,0,1.000000)!important;",
        "display:inline!important;",
        "opacity:1!important;",
        "visibility:visible!important;",
        "font-size:16px!important;",
        "font-family:serif!important;",
        "font-style:normal!important;",
        "font-weight:400!important;",
    ] {
        assert!(css.contains(declaration), "{css}");
    }
    let properties: Vec<_> = keys
        .iter()
        .map(|&property| SvgStyleProperty::new(property, true, None, None))
        .collect();
    let mut budget = 4096;
    let css = node_declarations(
        &ComputedValues::initial(),
        &properties,
        &[],
        false,
        &mut budget,
    )
    .unwrap();
    // Inherited font sizes use `100%` so renderers do not reapply a relative
    // ancestor size.
    assert_eq!(css.matches(":inherit!important;").count(), 7, "{css}");
    assert!(css.contains("font-size:100%!important;"), "{css}");
}

#[test]
fn descendant_declarations_carry_painting_properties_as_declared() {
    let painting = [
        (PropertyKey::Fill, "fill", "rgba(255,0,0,1)"),
        (PropertyKey::Stroke, "stroke", "url(\"#g\") none"),
        (PropertyKey::FillOpacity, "fill-opacity", "0.5"),
        (PropertyKey::StrokeOpacity, "stroke-opacity", "0.25"),
        (PropertyKey::StrokeWidth, "stroke-width", "3px"),
        (PropertyKey::FillRule, "fill-rule", "evenodd"),
        (PropertyKey::ClipRule, "clip-rule", "evenodd"),
        (PropertyKey::StrokeLinecap, "stroke-linecap", "round"),
        (PropertyKey::StrokeLinejoin, "stroke-linejoin", "miter-clip"),
        (PropertyKey::StrokeMiterlimit, "stroke-miterlimit", "2.5"),
        (PropertyKey::StrokeDasharray, "stroke-dasharray", "4 2"),
        (PropertyKey::StrokeDashoffset, "stroke-dashoffset", "-1px"),
    ];
    for (property, name, value) in painting {
        let declared = [SvgStyleProperty::new(
            property,
            false,
            Some(value.into()),
            None,
        )];
        let mut budget = 1024;
        assert_eq!(
            node_declarations(
                &ComputedValues::initial(),
                &declared,
                &[],
                false,
                &mut budget
            )
            .unwrap(),
            format!("{name}:{value}!important;")
        );

        let inherited = [SvgStyleProperty::new(property, true, None, None)];
        let mut budget = 1024;
        assert_eq!(
            node_declarations(
                &ComputedValues::initial(),
                &inherited,
                &[],
                false,
                &mut budget
            )
            .unwrap(),
            format!("{name}:inherit!important;")
        );

        // No computed field backs these properties, so a declaration without
        // its text has nothing to export.
        let missing = [SvgStyleProperty::new(property, false, None, None)];
        let mut budget = 1024;
        assert_eq!(
            node_declarations(
                &ComputedValues::initial(),
                &missing,
                &[],
                false,
                &mut budget
            )
            .unwrap(),
            ""
        );
    }
}

#[test]
fn descendant_declarations_carry_the_geometry_of_a_shape_as_declared() {
    let geometry = [
        (PropertyKey::SvgX, "x", "1px"),
        (PropertyKey::SvgY, "y", "2%"),
        (PropertyKey::SvgCx, "cx", "3em"),
        (PropertyKey::SvgCy, "cy", "-4px"),
        (PropertyKey::SvgR, "r", "5px"),
        (PropertyKey::SvgRx, "rx", "auto"),
        (PropertyKey::SvgRy, "ry", "6pt"),
        (PropertyKey::SvgD, "d", "path(\"M0 0\")"),
        (PropertyKey::Width, "width", "20px"),
        (PropertyKey::Height, "height", "auto"),
    ];
    for (property, name, value) in geometry {
        let declared = [SvgStyleProperty::new(
            property,
            false,
            Some(value.into()),
            None,
        )];
        let mut budget = 1024;
        assert_eq!(
            node_declarations(
                &ComputedValues::initial(),
                &declared,
                &[],
                false,
                &mut budget
            )
            .unwrap(),
            format!("{name}:{value}!important;")
        );
        // The text and a copy of it count against the budget, as they do for
        // a painting property.
        assert_eq!(budget, 1024 - 128 - 128 - 2 * value.len());

        // A geometry property is not inherited, so a literal `inherit`
        // declares nothing; and a declaration without text has nothing to
        // export either.
        for (inherited, expression) in [(true, None), (true, Some(value.into())), (false, None)] {
            let skipped = [SvgStyleProperty::new(property, inherited, expression, None)];
            let mut budget = 1024;
            assert_eq!(
                node_declarations(
                    &ComputedValues::initial(),
                    &skipped,
                    &[],
                    false,
                    &mut budget
                )
                .unwrap(),
                "",
                "{name}"
            );
        }
    }
}

#[test]
fn relative_presentation_attributes_resolve_against_the_declaring_font_size() {
    let attributes = [
        SvgStyleProperty::new(PropertyKey::StrokeWidth, false, Some("2em".into()), None),
        SvgStyleProperty::new(
            PropertyKey::StrokeDasharray,
            false,
            Some("2em 1ex".into()),
            None,
        ),
        SvgStyleProperty::new(
            PropertyKey::StrokeDashoffset,
            false,
            Some("-1em".into()),
            None,
        ),
    ];
    let mut style = ComputedValues::initial();
    style.font_size.0 = 20.0;
    let mut budget = 4096;
    assert_eq!(
        node_declarations(&style, &[], &attributes, false, &mut budget).unwrap(),
        "stroke-width:40px!important;stroke-dasharray:40px 10px!important;stroke-dashoffset:-20px!important;"
    );
}

#[test]
fn absolute_presentation_attributes_need_no_declaration() {
    let attributes = [SvgStyleProperty::new(
        PropertyKey::StrokeWidth,
        false,
        Some("3px".into()),
        None,
    )];
    let mut budget = 4096;
    assert_eq!(
        node_declarations(
            &ComputedValues::initial(),
            &[],
            &attributes,
            false,
            &mut budget
        )
        .unwrap(),
        ""
    );
}

#[test]
fn declared_font_relative_stroke_lengths_are_written_in_px_outside_an_instance() {
    let declared = [
        SvgStyleProperty::new(PropertyKey::StrokeWidth, false, Some("2em".into()), None),
        SvgStyleProperty::new(
            PropertyKey::StrokeDasharray,
            false,
            Some("2em 50% 1ex 3".into()),
            None,
        ),
        SvgStyleProperty::new(
            PropertyKey::StrokeDashoffset,
            false,
            Some("-1em".into()),
            None,
        ),
        SvgStyleProperty::new(
            PropertyKey::StrokeLinecap,
            false,
            Some("round".into()),
            None,
        ),
    ];
    let mut style = ComputedValues::initial();
    style.font_size.0 = 20.0;
    let mut budget = 4096;
    assert_eq!(
        node_declarations(&style, &declared, &[], false, &mut budget).unwrap(),
        "stroke-width:40px!important;stroke-dasharray:40px 50% 10px 3!important;stroke-dashoffset:-20px!important;stroke-linecap:round!important;"
    );
    // An instance resolves the same text against its own font size.
    let mut budget = 4096;
    assert_eq!(
        node_declarations(&style, &declared, &[], true, &mut budget).unwrap(),
        "stroke-width:2em!important;stroke-dasharray:2em 50% 1ex 3!important;stroke-dashoffset:-1em!important;stroke-linecap:round!important;"
    );
}

#[test]
fn the_px_text_of_a_stroke_length_counts_against_the_budget() {
    let declared = [SvgStyleProperty::new(
        PropertyKey::StrokeWidth,
        false,
        Some("2em".into()),
        None,
    )];
    let mut style = ComputedValues::initial();
    style.font_size.0 = 20.0;
    // The declaration's own 128 bytes, the text's 128 and its two copies, and
    // two copies of the `40px` that replaces it.
    let needed = 128 + (128 + 2 * 3) + 2 * 4;
    let mut remaining = needed - 1;
    assert!(node_declarations(&style, &declared, &[], false, &mut remaining).is_err());
    let mut remaining = needed;
    node_declarations(&style, &declared, &[], false, &mut remaining).unwrap();
    assert_eq!(remaining, 0);
}

#[test]
fn an_inherited_stroke_length_stays_inherit_in_any_place() {
    let inherited = [SvgStyleProperty::new(
        PropertyKey::StrokeWidth,
        true,
        Some("2em".into()),
        None,
    )];
    let mut style = ComputedValues::initial();
    style.font_size.0 = 20.0;
    for in_instance in [false, true] {
        let mut budget = 4096;
        assert_eq!(
            node_declarations(&style, &inherited, &[], in_instance, &mut budget).unwrap(),
            "stroke-width:inherit!important;"
        );
    }
}

#[test]
fn a_local_reference_is_read_as_the_renderer_reads_it() {
    assert_eq!(iri_fragment("#shape"), Some("shape"));
    assert_eq!(iri_fragment(" \n#shape  "), Some("shape"));
    // Only a space ends the id.
    assert_eq!(iri_fragment("#a\tb"), Some("a\tb"));
    for unreadable in ["", "#", "shape", "shape#", "#a b", "file.svg#shape"] {
        assert_eq!(iri_fragment(unreadable), None, "{unreadable:?}");
    }
}

/// Make `node` an SVG element holding `attributes`.
fn svg_element(doc: &mut Document, node: usize, attributes: &[(&str, &str)]) -> usize {
    doc.set_element_namespace(node, Some(SmolStr::new(SVG_NAMESPACE)));
    doc.set_element_attributes(
        node,
        attributes
            .iter()
            .map(|&(name, value)| (SmolStr::new(name), SmolStr::new(value)))
            .collect(),
    );
    node
}

/// How many parent links are followed to declare an inline SVG that a `use`
/// element instantiates a group of, `levels` deep. Each level is a `g` that
/// declares a stroke length in em at 20px, holding a `path` that inherits it at
/// 10px and the next level. The path takes the length from the `g` above it,
/// which is inside the group.
fn parent_steps_for_a_chain_of_levels(levels: usize) -> usize {
    let mut paths = Vec::new();
    let fixture = sheet_fixture(
        "g{font-size:20px;stroke-width:2em}path{font-size:10px;stroke-width:inherit}",
        "",
        |doc, root| {
            let svg = doc.append_element(Some(root), "svg", None::<&str>);
            svg_element(doc, svg, &[("width", "50"), ("height", "50")]);
            let defs = doc.append_element(Some(svg), "defs", None::<&str>);
            svg_element(doc, defs, &[]);
            let mut above = doc.append_element(Some(defs), "g", None::<&str>);
            svg_element(doc, above, &[("id", "d")]);
            for _ in 0..levels {
                let path = doc.append_element(Some(above), "path", None::<&str>);
                svg_element(doc, path, &[]);
                paths.push(path);
                above = doc.append_element(Some(above), "g", None::<&str>);
                svg_element(doc, above, &[]);
            }
            let instance = doc.append_element(Some(svg), "use", None::<&str>);
            svg_element(doc, instance, &[("href", "#d")]);
        },
    );
    let svg = fixture.doc.get_node(fixture.root).unwrap().children[0];
    INLINE_SVG_PARENT_STEPS.with(|steps| steps.set(0));
    let elements = inline_svg_element_declarations(&fixture.doc, &fixture.cascade, svg).unwrap();
    let steps = INLINE_SVG_PARENT_STEPS.with(|steps| steps.get());
    // Every path takes its length from a group inside the instance, which the
    // renderer finds by itself: only the font size of the path is written.
    for path in paths {
        let index = path_index(&fixture.doc, svg, path);
        let listed = elements
            .iter()
            .find(|element| element.element_index == index)
            .unwrap_or_else(|| panic!("no declarations for the path at index {index}"));
        assert_eq!(listed.declarations, "font-size:10px!important;");
    }
    steps
}

/// The preorder index of the element `node` in the SVG `svg`, counting `svg`
/// as zero.
fn path_index(doc: &Document, svg: usize, node: usize) -> usize {
    let mut stack = vec![svg];
    let mut index = 0;
    while let Some(id) = stack.pop() {
        if id == node {
            return index;
        }
        if doc.get_node(id).unwrap().kind() == NodeKind::Element {
            index += 1;
        }
        stack.extend(doc.get_node(id).unwrap().children.iter().rev());
    }
    panic!("{node} is not in the SVG");
}

#[test]
fn an_inherited_length_is_placed_in_an_instance_without_climbing_to_it_for_every_level() {
    let few = parent_steps_for_a_chain_of_levels(32);
    let many = parent_steps_for_a_chain_of_levels(128);
    // The group is found for each level by where it sits among the elements
    // above the path.
    assert!(few <= 32 * 8, "{few} steps for 32 levels");
    assert!(many <= 128 * 8, "{many} steps for 128 levels");
}

#[test]
fn the_ancestors_of_an_element_tell_where_other_elements_are_without_climbing() {
    let mut doc = Document::new();
    let top = doc.append_element(Some(0), "a", None::<&str>);
    let walk_root = doc.append_element(Some(top), "b", None::<&str>);
    let left = doc.append_element(Some(walk_root), "c", None::<&str>);
    let left_child = doc.append_element(Some(left), "d", None::<&str>);
    let right = doc.append_element(Some(walk_root), "e", None::<&str>);
    // At `left_child`, on the way from `walk_root`.
    let mut ancestors = Ancestors::above(&doc, walk_root);
    ancestors.enter(walk_root, 0);
    ancestors.enter(left, 1);
    ancestors.enter(left_child, 2);
    INLINE_SVG_PARENT_STEPS.with(|steps| steps.set(0));
    for (node, ancestor, within) in [
        (left_child, left, true),
        (left, left, true),
        (left, left_child, false),
        (left_child, walk_root, true),
        // An element above the root of the walk.
        (top, walk_root, false),
        (left, top, true),
    ] {
        assert_eq!(ancestors.is_within(&doc, node, ancestor), within);
    }
    assert_eq!(INLINE_SVG_PARENT_STEPS.with(|steps| steps.get()), 0);
    // Moving to the next branch drops the elements of the one that was left.
    ancestors.enter(right, 1);
    assert!(ancestors.is_within(&doc, right, walk_root));
    assert!(!ancestors.position.contains_key(&left));
    assert!(!ancestors.position.contains_key(&left_child));
    // An element that is not on the path is searched for in the tree.
    assert!(ancestors.is_within(&doc, left_child, left));
    assert!(!ancestors.is_within(&doc, left_child, right));
    assert!(INLINE_SVG_PARENT_STEPS.with(|steps| steps.get()) > 0);
}

#[test]
fn relative_presentation_attributes_of_an_instantiated_element_stay_in_the_source() {
    let attributes = [
        SvgStyleProperty::new(PropertyKey::StrokeWidth, false, Some("2em".into()), None),
        SvgStyleProperty::new(
            PropertyKey::StrokeDasharray,
            false,
            Some("2em 1ex".into()),
            None,
        ),
    ];
    let mut style = ComputedValues::initial();
    style.font_size.0 = 20.0;
    let mut budget = 4096;
    // Each instance resolves the text of the attribute at its own font size.
    assert_eq!(
        node_declarations(&style, &[], &attributes, true, &mut budget).unwrap(),
        ""
    );
}

#[test]
fn a_stroke_length_an_instantiated_element_inherits_is_left_to_the_renderer() {
    let declarer = Some(StyleNodeId(7));
    let properties = [
        SvgStyleProperty::new(PropertyKey::StrokeWidth, true, None, None),
        // The cascade wrote the length of an ancestor in px.
        SvgStyleProperty::new(
            PropertyKey::StrokeDasharray,
            false,
            Some("40px 10px".into()),
            declarer,
        ),
        SvgStyleProperty::new(PropertyKey::StrokeDashoffset, true, None, None),
        // Only a stroke length is left out: the others are copied as they are.
        SvgStyleProperty::new(PropertyKey::Fill, true, None, None),
        SvgStyleProperty::new(
            PropertyKey::StrokeLinecap,
            false,
            Some("round".into()),
            None,
        ),
    ];
    let style = ComputedValues::initial();
    let mut budget = 4096;
    assert_eq!(
        node_declarations_at(
            &style,
            &properties,
            &[],
            Placement::InstanceInheritingByItself,
            &mut budget
        )
        .unwrap(),
        "fill:inherit!important;stroke-linecap:round!important;"
    );
    // Without the permission, the declarations stay.
    let mut budget = 4096;
    assert_eq!(
        node_declarations_at(&style, &properties, &[], Placement::Instance, &mut budget).unwrap(),
        "stroke-width:inherit!important;stroke-dasharray:40px 10px!important;stroke-dashoffset:inherit!important;fill:inherit!important;stroke-linecap:round!important;"
    );
}

/// The declarations listed for the `path` of an SVG whose group `g#d` declares
/// a stroke width of 2em at 20px and holds the `path`, which inherits it at
/// 10px, with an `!important` rule that the source of the `path` cannot beat.
/// The `path` has `path_attributes` and the `style` attribute `path_style`. A `use` element instantiates the group when `instantiated` is
/// set, and the SVG holds a `style` element with the text `inner_style` when
/// there is one.
fn declarations_of_a_path_inheriting_a_stroke_width(
    path_attributes: &[(&str, &str)],
    path_style: Option<&str>,
    inner_style: Option<&str>,
    instantiated: bool,
) -> String {
    let mut path = 0;
    let fixture = sheet_fixture(
        "g{font-size:20px;stroke-width:2em}path{font-size:10px;stroke-width:inherit!important}",
        "",
        |doc, root| {
            let svg = doc.append_element(Some(root), "svg", None::<&str>);
            svg_element(doc, svg, &[("width", "50"), ("height", "50")]);
            if let Some(text) = inner_style {
                let style = doc.append_element(Some(svg), "style", None::<&str>);
                svg_element(doc, style, &[]);
                doc.append_text(style, text);
            }
            let group = doc.append_element(Some(svg), "g", None::<&str>);
            svg_element(doc, group, &[("id", "d")]);
            path = doc.append_element(Some(group), "path", path_style);
            svg_element(doc, path, path_attributes);
            if instantiated {
                let instance = doc.append_element(Some(svg), "use", None::<&str>);
                svg_element(doc, instance, &[("href", "#d")]);
            }
        },
    );
    let svg = fixture.doc.get_node(fixture.root).unwrap().children[0];
    let elements = inline_svg_element_declarations(&fixture.doc, &fixture.cascade, svg).unwrap();
    let index = path_index(&fixture.doc, svg, path);
    elements
        .into_iter()
        .find(|element| element.element_index == index)
        .map(|element| element.declarations)
        .unwrap_or_default()
}

#[test]
fn an_inherited_stroke_width_is_left_out_only_where_the_source_sets_none() {
    let declarations = declarations_of_a_path_inheriting_a_stroke_width;
    // The renderer finds the width of the group, at the font size of the
    // group in each instance.
    assert_eq!(
        declarations(&[], None, None, true),
        "font-size:10px!important;"
    );
    // Outside an instance the width is written in px, as the font size of the
    // group is known.
    assert_eq!(
        declarations(&[], None, None, false),
        "font-size:10px!important;stroke-width:40px!important;"
    );
    // Text of the source that sets a stroke length would win over a width that
    // is not written.
    for (attributes, style) in [
        (&[("stroke-width", "3")][..], None),
        (&[("stroke-dasharray", "1")], None),
        (&[("stroke-dashoffset", "1")], None),
    ] {
        assert!(
            declarations(attributes, style, None, true).contains("stroke-width:40px!important;"),
            "{attributes:?} {style:?}"
        );
    }
    // The text that can name a stroke property is not read closely: an escape
    // or a comment may spell the name.
    for style in [
        Some("Stroke-Linecap: round"),
        Some("\\73troke-width: 3px"),
        Some("stro/**/ke-width: 3px"),
    ] {
        assert!(
            declarations(&[], style, None, true).contains("stroke-width:40px!important;"),
            "{style:?}"
        );
    }
    // So would a style sheet in the SVG.
    for sheet in ["path { stroke-width: 3px }", "path { STROKE-DASHARRAY: 1 }"] {
        assert!(
            declarations(&[], None, Some(sheet), true).contains("stroke-width:40px!important;"),
            "{sheet}"
        );
    }
    // Text about something else changes nothing.
    assert!(!declarations(&[], Some("fill: red"), None, true).contains("stroke-width"));
    assert!(!declarations(&[], None, Some("path { fill: red }"), true).contains("stroke-width"));
}
