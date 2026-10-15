use super::*;

#[test]
fn all_vertical_modes_use_the_containing_block_inline_percentage_basis() {
    use raikiri_style::property::WritingMode;
    for (mode, expected) in [
        (WritingMode::HorizontalTb, 60.0),
        (WritingMode::VerticalRl, 30.0),
        (WritingMode::VerticalLr, 30.0),
        (WritingMode::SidewaysRl, 30.0),
        (WritingMode::SidewaysLr, 30.0),
    ] {
        let mut tree = Document::new();
        let parent = tree.append_element(Some(0), "div", Style::default(), None::<&str>);
        let node = tree.append_element(Some(parent), "div", Style::default(), None::<&str>);
        tree.nodes[parent].used_writing_mode = mode;

        assert_eq!(
            ifc_parent_inline_size(
                &tree,
                node,
                Size {
                    width: Some(60.0),
                    height: Some(30.0),
                },
            ),
            Some(expected),
            "{mode:?} uses its containing block inline basis"
        );
    }
}

#[test]
fn whitespace_only_text_is_not_a_child_of_a_block_container() {
    // CSS 2.1 9.2.1.1: a whitespace-only run between blocks generates no
    // anonymous block box, so taffy's block algorithm never sees it and it
    // cannot separate the margins of its siblings.
    let mut tree = Document::new();
    let parent = tree.append_element(Some(0), "div", Style::default(), None::<&str>);
    tree.nodes[parent].style.display = Display::Block;
    tree.append_text(parent, "\n  ");
    let first = tree.append_element(Some(parent), "div", Style::default(), None::<&str>);
    tree.append_text(parent, "\t\r\u{000c} ");
    let text = tree.append_text(parent, " x ");
    let last = tree.append_element(Some(parent), "div", Style::default(), None::<&str>);
    tree.append_text(parent, "\n");
    let parent = NodeId::from(parent);

    let children: Vec<usize> = tree.child_ids(parent).map(usize::from).collect();
    assert_eq!(children, vec![first, text, last]);
    assert_eq!(tree.child_count(parent), 3);
    assert_eq!(usize::from(tree.get_child_id(parent, 1)), text);
}

#[test]
fn a_percentage_max_height_of_an_indefinite_basis_does_not_bound_the_height() {
    // CSS 2.1 10.7: a percentage `max-height` against an indefinite
    // containing block height is treated as `none`.
    let style = Style {
        size: Size {
            width: taffy::Dimension::auto(),
            height: taffy::Dimension::length(100.0),
        },
        max_size: Size {
            width: taffy::LengthPercentageAuto::auto(),
            height: taffy::LengthPercentageAuto::percent(0.5),
        },
        ..Style::default()
    };
    let bounds = ifc_content_height_bounds(&style, None, None);
    assert_eq!(bounds, (0.0, f32::INFINITY));
    assert_eq!(ifc_content_height_bounds(&style, Some(80.0), None).1, 40.0);

    // A calc() bound with a percentage needs the basis as well.
    let mut arena = Vec::new();
    let calc_ptr = push_calc(
        &mut arena,
        raikiri_style::property::CalcLengthPercentage {
            percent: 50.0,
            px: 2.0,
        },
        false,
    );
    let style = Style {
        max_size: Size {
            width: taffy::LengthPercentageAuto::auto(),
            height: taffy::LengthPercentageAuto::calc(calc_ptr),
        },
        ..style
    };
    assert_eq!(
        ifc_content_height_bounds(&style, None, None),
        (0.0, f32::INFINITY)
    );
    assert_eq!(ifc_content_height_bounds(&style, Some(80.0), None).1, 42.0);

    // A calc() bound without a percentage needs no basis.
    let length_ptr = push_calc(
        &mut arena,
        raikiri_style::property::CalcLengthPercentage {
            percent: 0.0,
            px: 30.0,
        },
        false,
    );
    let length = taffy::LengthPercentageAuto::calc(length_ptr);
    let style = Style {
        max_size: Size {
            width: taffy::LengthPercentageAuto::auto(),
            height: length,
        },
        ..style
    };
    assert_eq!(ifc_content_height_bounds(&style, None, None), (0.0, 30.0));
    assert!(ifc_length_only(length).into_raw().is_calc());
    assert!(
        ifc_length_only(taffy::LengthPercentageAuto::calc(calc_ptr))
            .into_raw()
            .tag()
            == taffy::CompactLength::AUTO_TAG
    );
}

#[test]
fn a_horizontal_override_keeps_the_vertical_ancestor_fragmentation_guard() {
    use crate::layout::ifc::boxes::IfcBoxKind;
    use crate::layout::test_support::ifc_ahem_fonts;
    for mode in [
        "horizontal-tb",
        "vertical-rl",
        "vertical-lr",
        "sideways-rl",
        "sideways-lr",
    ] {
        let mut doc = Document::new();
        let body = doc.append_element(Some(0), "body", Style::default(), Some("display:block"));
        let ancestor = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some(format!("display:block;writing-mode:{mode}")),
        );
        let columns = doc.append_element(Some(ancestor), "div", Style::default(),
            Some("display:block;writing-mode:horizontal-tb;columns:2;column-gap:0;width:200px;height:100px"));
        let flex = doc.append_element(Some(columns), "div", Style::default(), Some("display:flex"));
        let paragraph = doc.append_element(
            Some(flex),
            "div",
            Style::default(),
            Some("display:block;font:10px Ahem"),
        );
        let float = doc.append_element(
            Some(paragraph),
            "div",
            Style::default(),
            Some("display:block;float:left;width:10px;height:10px"),
        );
        doc.append_text(float, "x");
        doc.append_text(paragraph, "text");
        doc.mark_in_document_flags();
        doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
        let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
        crate::layout::apply_computed_to_style(&mut doc, &cascade).unwrap();
        let root = doc.nodes[paragraph].ifc.as_ref().unwrap().clone();
        assert!(root.boxes.iter().any(|box_| box_.kind == IfcBoxKind::Float));
        assert_eq!(
            doc.nodes[paragraph].used_writing_mode,
            raikiri_style::property::WritingMode::HorizontalTb
        );
        let context = crate::fragment::FragmentationContext::resolve(
            200.0,
            Some(100.0),
            doc.nodes[columns].multicol.unwrap(),
        )
        .unwrap();
        doc.fragmentation_stack.push(context);
        let expected = (mode == "horizontal-tb").then_some(context);
        assert_eq!(
            fragmentation_context_for_ifc(&doc, paragraph, &root),
            expected
        );
    }
}
