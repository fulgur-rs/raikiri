use super::*;
use crate::layout::test_support::{cascaded, with_bridged_session, with_resumed_session};
use crate::node::FormattingContext;

/// Bridge the computed styles of `doc`, as layout does before it queries them.
fn bridged(doc: &mut Document) -> raikiri_style::CascadeResult {
    let rules = raikiri_style::build_rule_tree(doc);
    let cascade = raikiri_style::cascade(doc, &rules).expect("cascade Ok");
    apply_computed_to_style(doc, &cascade).expect("styles");
    cascade
}

#[test]
fn single_paragraph_balance_preserves_child_decoration_and_constraint_guards() {
    // A laid-out multicol container with one paragraph, whose boxes carry
    // `parent_css` and `child_css`.
    let laid_out = |parent_css: &str, child_css: &str| {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", None::<&str>);
        let body = doc.append_element(Some(html), "body", Some("margin:0"));
        let parent = doc.append_element(
            Some(body),
            "div",
            Some(
                format!("display:block;width:100px;column-count:2;column-gap:20px;{parent_css}")
                    .as_str(),
            ),
        );
        let child = doc.append_element(
            Some(parent),
            "p",
            Some(format!("display:block;margin:0;{child_css}").as_str()),
        );
        doc.append_text(child, "text");
        layout_nested_flex_float_fixture(&mut doc);
        (doc, child)
    };
    let balances = |parent_css: &str, child_css: &str| {
        let (mut doc, child) = laid_out(parent_css, child_css);
        with_resumed_session(&mut doc, |session| {
            can_balance_single_paragraph(session, child)
        })
    };
    assert!(balances("", ""));
    // A height, a minimum height or a maximum height on the paragraph keeps
    // it whole.
    assert!(!balances("", "height:80px"));
    assert!(!balances("", "min-height:80px"));
    assert!(!balances("", "max-height:80px"));

    let (mut doc, child) = laid_out("", "");
    doc.nodes[child].break_inside = raikiri_style::property::BreakInside::Avoid;
    assert!(!with_resumed_session(&mut doc, |session| {
        can_balance_single_paragraph(session, child)
    }));
    doc.nodes[child].break_inside = raikiri_style::property::BreakInside::Auto;
    doc.nodes[child].layout.padding.top = 4.0;
    assert!(!with_resumed_session(&mut doc, |session| {
        can_balance_single_paragraph(session, child)
    }));
    doc.nodes[child].layout.padding.top = 0.0;
    doc.nodes[child].layout.border.bottom = 4.0;
    assert!(!with_resumed_session(&mut doc, |session| {
        can_balance_single_paragraph(session, child)
    }));
    doc.nodes[child].layout.border.bottom = 0.0;
    // The container's own padding, border and direction are no concern of
    // its paragraph.
    assert!(balances("padding-left:4px", ""));
    assert!(balances("padding-left:4px;border-top:1px solid", ""));
    assert!(balances("direction:rtl", ""));
}

#[test]
fn balanced_column_height_rejects_empty_unavailable_or_invalid_inputs() {
    let context = FragmentationContext {
        rtl: false,
        available_width: 200.0,
        available_height: None,
        column_fill: raikiri_style::property::ColumnFillValue::Balance,
        column_width: 100.0,
        column_count: 2,
        column_gap: 0.0,
        column_index: 0,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 1,
    };

    assert_eq!(balanced_column_height(&[], context), None);
    assert_eq!(
        balanced_column_height(&[(0.0, 10.0)], context.in_column(2, 200.0, 0.0)),
        None
    );
    assert_eq!(balanced_column_height(&[(10.0, 0.0)], context), None);
}

#[test]
fn line_ranges_in_columns_preserves_offsets_and_satisfies_widows() {
    let context = FragmentationContext {
        rtl: false,
        available_width: 300.0,
        available_height: Some(100.0),
        column_fill: raikiri_style::property::ColumnFillValue::Balance,
        column_width: 100.0,
        column_count: 3,
        column_gap: 0.0,
        column_index: 1,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 2,
    };
    let extents = [
        (0.0, 10.0),
        (20.0, 30.0),
        (40.0, 50.0),
        (60.0, 70.0),
        (100.0, 110.0),
    ];

    assert_eq!(
        line_ranges_in_columns(&extents, context, true),
        vec![(0, 3, 1), (3, 5, 2)],
    );
}

#[test]
fn line_ranges_in_columns_balances_when_height_is_indefinite_and_handles_empty_columns() {
    let context = FragmentationContext {
        rtl: false,
        available_width: 300.0,
        available_height: None,
        column_fill: raikiri_style::property::ColumnFillValue::Balance,
        column_width: 100.0,
        column_count: 3,
        column_gap: 0.0,
        column_index: 0,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 1,
    };
    let extents = [
        (0.0, 10.0),
        (20.0, 30.0),
        (40.0, 50.0),
        (60.0, 70.0),
        (80.0, 90.0),
    ];

    assert_eq!(
        line_ranges_in_columns(&extents, context, false),
        vec![(0, 2, 0), (2, 4, 1), (4, 5, 2)],
    );
    assert!(line_ranges_in_columns(&[], context, false).is_empty());
    assert!(line_ranges_in_columns(&extents, context.in_column(3, 300.0, 0.0), false).is_empty());
    assert!(line_ranges_in_columns(&extents, context.in_column(3, 300.0, 0.0), true).is_empty());
    // Past the last column of a definite-height container, the lines continue
    // through as many overflow columns as they need.
    let definite = FragmentationContext {
        available_height: Some(30.0),
        ..context
    };
    assert_eq!(
        line_ranges_in_columns(&extents, definite.in_column(3, 300.0, 0.0), false),
        vec![(0, 2, 3), (2, 4, 4), (4, 5, 5)],
    );
    // Lines that start in the declared columns continue past them too.
    let short = FragmentationContext {
        available_height: Some(20.0),
        ..context
    };
    assert_eq!(
        line_ranges_in_columns(&extents, short, false),
        vec![(0, 1, 0), (1, 2, 1), (2, 3, 2), (3, 4, 3), (4, 5, 4)],
    );
}

#[test]
fn multicol_subtree_has_float_skips_hidden_descendants() {
    let mut doc = Document::new();
    let root = doc.append_element(Some(0), "div", None::<&str>);
    let hidden = doc.append_element(Some(root), "div", None::<&str>);
    doc.append_element(Some(hidden), "div", Some("float:left"));
    bridged(&mut doc);
    doc.set_display(hidden, raikiri_style::property::DisplayValue::None);
    doc.nodes[hidden].formatting_context = FormattingContext::None;

    assert!(!with_resumed_session(&mut doc, |session| {
        multicol_subtree_has_float(session, root)
    }));

    doc.set_display(hidden, raikiri_style::property::DisplayValue::Block);
    doc.nodes[hidden].formatting_context = FormattingContext::Block;
    assert!(with_resumed_session(&mut doc, |session| {
        multicol_subtree_has_float(session, root)
    }));
}

#[test]
fn only_a_lone_trailing_float_of_a_block_container_ends_with_its_only_float() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let block = "display:block;height:20px";
    let float = "display:block;float:right;width:20px;height:20px";
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let mut container = |children: &[&str], text: Option<&str>| {
        let container = doc.append_element(
            Some(body),
            "div",
            Some("display:block;column-count:2;width:100px;height:40px;font:20px/20px Ahem"),
        );
        if let Some(text) = text {
            doc.append_text(container, text);
        }
        for css in children {
            doc.append_element(Some(container), "div", Some(*css));
        }
        container
    };
    let empty = container(&[], None);
    let alone = container(&[float], None);
    let trailing = container(&[block, float], None);
    let leading = container(&[float, block], None);
    let two = container(&[float, float], None);
    let inline = container(&[float], Some("A B"));
    let nested = container(&[block, float], None);
    let margin = container(
        &["display:block;height:20px;margin-bottom:20px", float],
        None,
    );
    let nested_margin = container(&[block, float], None);
    let hidden = container(&[block, float], None);
    let wrapper = doc.nodes[nested].children[0];
    doc.append_element(Some(wrapper), "div", Some(float));
    let wrapper = doc.nodes[nested_margin].children[0];
    doc.append_element(Some(wrapper), "div", Some("display:block;margin-top:20px"));
    let wrapper = doc.nodes[hidden].children[0];
    doc.append_element(
        Some(wrapper),
        "div",
        Some("display:none;float:left;margin-top:20px"),
    );
    doc.set_font_collection(crate::layout::test_support::ifc_ahem_fonts());

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 300.0;
    page.height = 400.0;
    layout_single_page(&mut doc, &cascade, page).expect("layout Ok");

    assert!(!with_bridged_session(&mut doc, |session| {
        multicol_ends_with_its_only_float(session, empty)
    }));
    assert!(with_bridged_session(&mut doc, |session| {
        multicol_ends_with_its_only_float(session, alone)
    }));
    assert!(with_bridged_session(&mut doc, |session| {
        multicol_ends_with_its_only_float(session, trailing)
    }));
    // Content after the float wraps beside it, and a second float shares its
    // line, also when that float is inside an earlier child.
    assert!(!with_bridged_session(&mut doc, |session| {
        multicol_ends_with_its_only_float(session, leading)
    }));
    assert!(!with_bridged_session(&mut doc, |session| {
        multicol_ends_with_its_only_float(session, two)
    }));
    assert!(!with_bridged_session(&mut doc, |session| {
        multicol_ends_with_its_only_float(session, nested)
    }));
    // Adjoining margins collapse on the block layout, also through a wrapper.
    assert!(!with_bridged_session(&mut doc, |session| {
        multicol_ends_with_its_only_float(session, margin)
    }));
    assert!(!with_bridged_session(&mut doc, |session| {
        multicol_ends_with_its_only_float(session, nested_margin)
    }));
    // Hidden content takes no part in either.
    assert!(with_bridged_session(&mut doc, |session| {
        multicol_ends_with_its_only_float(session, hidden)
    }));
    // Lines of the container's own are laid out beside the float.
    assert!(doc.nodes[inline].ifc.is_some());
    assert!(!with_bridged_session(&mut doc, |session| {
        multicol_ends_with_its_only_float(session, inline)
    }));
}

#[test]
fn flex_float_chain_is_not_a_scope_without_a_multicol_ancestor() {
    let mut doc = Document::new();
    let node = doc.append_element(Some(0), "div", None::<&str>);

    assert!(
        !with_bridged_session(&mut doc, |session| FlexFloatChain::of_ancestors(
            session, node
        ))
        .is_scope()
    );
}

// The chain carried down a traversal must agree with walking every node's
// ancestors, for every arrangement of the boxes that end or extend a chain.
#[test]
fn flex_float_chain_matches_an_ancestor_walk() {
    /// What a box of the chain is: plain, a row or column flex container, a
    /// float, a vertical writing mode, or a horizontal or vertical multicol
    /// container.
    #[derive(Clone, Copy, PartialEq)]
    enum Kind {
        Plain,
        RowFlex,
        ColumnFlex,
        Float,
        Vertical,
        HorizontalMulticol,
        VerticalMulticol,
    }
    const KINDS: [Kind; 7] = [
        Kind::Plain,
        Kind::RowFlex,
        Kind::ColumnFlex,
        Kind::Float,
        Kind::Vertical,
        Kind::HorizontalMulticol,
        Kind::VerticalMulticol,
    ];
    // Walk the ancestors of the box at `depth` from the box itself up.
    fn walk(kinds: &[Kind], depth: usize) -> bool {
        let mut has_row_flex = false;
        let mut has_float = false;
        for kind in kinds[..=depth].iter().rev() {
            has_float |= *kind == Kind::Float;
            match kind {
                Kind::Vertical => return false,
                Kind::HorizontalMulticol | Kind::VerticalMulticol => {
                    return has_row_flex && has_float && *kind == Kind::HorizontalMulticol;
                }
                _ => {}
            }
            has_row_flex |= *kind == Kind::RowFlex;
        }
        false
    }
    for mut arrangement in 0..KINDS.len().pow(4) {
        let mut kinds = Vec::new();
        for _ in 0..4 {
            kinds.push(KINDS[arrangement % KINDS.len()]);
            arrangement /= KINDS.len();
        }
        let mut doc = Document::new();
        let mut parent = 0;
        let mut nodes = Vec::new();
        for kind in &kinds {
            let css = match kind {
                Kind::RowFlex => "display:flex",
                Kind::ColumnFlex => "display:flex;flex-direction:column",
                Kind::Float => "float:left",
                Kind::Vertical => "display:block;writing-mode:vertical-rl",
                Kind::HorizontalMulticol => "display:block;column-count:2",
                Kind::VerticalMulticol => "display:block;writing-mode:vertical-rl;column-count:2",
                Kind::Plain => "display:block",
            };
            let node = doc.append_element(Some(parent), "div", Some(css));
            nodes.push(node);
            parent = node;
        }
        let cascade = cascaded(&doc);
        let tree = LayoutSession::bridged(&mut doc, &cascade);
        let mut carried = FlexFloatChain::of_ancestors(&tree, 0);
        for (depth, &node) in nodes.iter().enumerate() {
            carried = carried.below(&tree, node);
            assert_eq!(carried.is_scope(), walk(&kinds, depth));
            assert_eq!(
                FlexFloatChain::of_ancestors(&tree, node).is_scope(),
                walk(&kinds, depth)
            );
        }
    }
}

#[test]
fn ancestor_facts_resolve_each_chain_once_from_the_top() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    crate::layout::test_support::with_ahem(&mut doc);
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let sized = doc.append_element(Some(body), "div", Some("width:400px"));
    let half = doc.append_element(Some(sized), "div", Some("width:50%"));
    let auto = doc.append_element(Some(half), "div", None::<&str>);
    let intrinsic = doc.append_element(
        Some(auto),
        "div",
        Some("width:min-content;position:absolute"),
    );
    let flex = doc.append_element(Some(intrinsic), "div", Some("display:flex"));
    let vertical = doc.append_element(Some(flex), "div", Some("writing-mode:vertical-rl"));
    let leaf = doc.append_element(Some(vertical), "div", None::<&str>);
    let late_parent = doc.append_element(Some(body), "div", Some("width:30px"));
    // Reparent under a later arena index and keep both DOM edge representations consistent.
    doc.detach_from_parent(half);
    doc.attach_child(late_parent, half);
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    crate::layout::bridge::apply_computed_to_style(&mut doc, &cascade).expect("bridge");
    let facts = AncestorFacts::new(&doc, &cascade, 800.0);

    assert_eq!(facts.content_width[sized], 400.0);
    assert_eq!(facts.content_width[half], 15.0);
    assert_eq!(facts.content_width[leaf], 15.0);
    assert_eq!(facts.authored_containing_width(half), Some(30.0));
    assert_eq!(facts.authored_containing_width(auto), Some(15.0));
    // `min-content` is intrinsic, so the search continues past it.
    assert_eq!(facts.authored_containing_width(flex), Some(15.0));
    assert!(!facts.out_of_flow_ancestor[intrinsic]);
    assert!(facts.out_of_flow_ancestor[flex]);
    assert!(!facts.non_block_flow_ancestor[flex]);
    assert!(facts.non_block_flow_ancestor[vertical]);
    assert!(!facts.vertical[flex]);
    assert!(facts.vertical[vertical] && facts.vertical[leaf]);
    assert!(!facts.multicol_ancestor[leaf]);
    assert_eq!(facts.authored_containing_width(html), None);
}

#[test]
fn avoid_page_min_block_child_can_continue_across_columns() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let multicol = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:200px;height:40px;column-count:2;column-gap:0"),
    );
    let child = doc.append_element(
        Some(multicol),
        "div",
        Some("display:block;min-block-size:20px;break-inside:avoid-page"),
    );
    for _ in 0..8 {
        doc.append_text(child, "line");
        doc.append_element(Some(child), "br", None::<&str>);
    }

    layout_nested_flex_float_fixture(&mut doc);
    assert!(!doc.nodes[child].has_logical_min_block_size);
    assert!(doc.nodes[child].is_ifc_root());
    let ranges = doc.nodes[child]
        .ifc_multicol_fragments()
        .expect("column line ranges");
    assert!(
        ranges.iter().any(|range| range.fragmentainer > 0),
        "avoid-page must allow the text to reach another column"
    );
}

#[test]
fn multicol_max_fragmentainer_height_applies_border_box_max_height() {
    let mut doc = Document::new();
    let node = doc.append_element(
        Some(0),
        "div",
        Some("box-sizing:border-box;max-height:100px"),
    );
    bridged(&mut doc);

    assert_eq!(
        with_bridged_session(&mut doc, |session| multicol_max_fragmentainer_height(
            session, node, 120.0, None, None
        )),
        Some(100.0),
    );
}

#[test]
fn multicol_min_constrained_child_requires_all_constraints() {
    let constrained = |child_css: &str| {
        let mut doc = Document::new();
        let parent = doc.append_element(Some(0), "div", None::<&str>);
        doc.append_element(Some(parent), "div", Some(child_css));
        with_bridged_session(&mut doc, |session| {
            multicol_has_min_constrained_child(session, parent)
        })
    };

    // The logical minimum counts once it wins over the physical one and the
    // box avoids breaks inside it.
    assert!(constrained("min-block-size:40px;break-inside:avoid"));
    assert!(!constrained("min-block-size:auto;break-inside:avoid"));
    assert!(!constrained("min-block-size:40px"));
    assert!(!constrained("min-height:40px;break-inside:avoid"));
}

#[test]
fn authored_containing_width_uses_the_cascade_resolved_ch_width() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let container = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:4ch;font-size:25px;font-family:monospace"),
    );
    let text = doc.append_text(container, "test");
    doc.set_font_collection(crate::fonts::system_font_collection());
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cascade).expect("styles");

    let taffy_width = with_bridged_session(&mut doc, |session| {
        style_dimension_length(style_of(session, container).size().width)
    })
    .expect("cascade-resolved ch width should be a finite length");
    assert_eq!(
        AncestorFacts::new(&doc, &cascade, 800.0).authored_containing_width(text),
        Some(taffy_width),
    );
}

#[test]
fn authored_containing_width_uses_content_box_for_border_box_ancestor() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let container = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:250px;box-sizing:border-box;border:3px solid"),
    );
    let text = doc.append_text(container, "generated text");
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cascade).expect("styles");

    assert_eq!(
        AncestorFacts::new(&doc, &cascade, 800.0).authored_containing_width(text),
        Some(244.0),
    );
}

#[test]
fn content_widths_resolve_a_cascade_ch_calc_with_a_percentage() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let container = doc.append_element(Some(body), "div", Some("display:block;width:200px"));
    let sized = doc.append_element(
        Some(container),
        "div",
        Some("display:block;width:calc(50% + 2ch);font-family:Ahem;font-size:10px"),
    );
    let text = doc.append_text(sized, "x");
    doc.set_font_collection(crate::layout::ifc::test_support::ahem_fonts());
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cascade).expect("styles");

    // Half of the 200px container plus two 10px Ahem "0" advances.
    assert_eq!(
        AncestorFacts::new(&doc, &cascade, 800.0).authored_containing_width(text),
        Some(120.0),
    );
    let facts = AncestorFacts::new(&doc, &cascade, 800.0);
    assert_eq!(facts.content_width[sized], 120.0);
}

#[test]
fn content_widths_skip_a_cascade_ch_width_on_display_contents() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let container = doc.append_element(Some(body), "div", Some("display:block;width:800px"));
    let contents = doc.append_element(
        Some(container),
        "div",
        Some("display:contents;width:calc(50% + 2ch);font-family:Ahem;font-size:10px"),
    );
    let half = doc.append_element(Some(contents), "div", Some("display:block;width:50%"));
    let text = doc.append_text(half, "x");
    doc.set_font_collection(crate::layout::ifc::test_support::ahem_fonts());
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cascade).expect("styles");

    // The contents element has no box, so the child halves the 800px container.
    assert_eq!(
        AncestorFacts::new(&doc, &cascade, 1000.0).authored_containing_width(text),
        Some(400.0),
    );
    let facts = AncestorFacts::new(&doc, &cascade, 1000.0);
    assert_eq!(facts.content_width[half], 400.0);
}

#[test]
fn authored_containing_width_preserves_percentage_fallback_behavior() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let container = doc.append_element(Some(body), "div", Some("display:block;width:200px"));
    let percentage = doc.append_element(Some(container), "div", Some("display:block;width:50%"));
    let text = doc.append_text(percentage, "percentage");
    let auto_text = doc.append_text(body, "auto");
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");

    assert_eq!(
        AncestorFacts::new(&doc, &cascade, 800.0).authored_containing_width(text),
        Some(100.0),
    );
    assert_eq!(
        AncestorFacts::new(&doc, &cascade, 800.0).authored_containing_width(auto_text),
        None,
    );
}

#[test]
fn compute_multicol_layout_moves_break_avoid_min_height_children_to_the_next_column() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let container = doc.append_element(
        Some(body),
        "div",
        Some("display:block;column-count:2;column-gap:20px;width:200px;height:60px"),
    );
    // Children that hold no lines are cut at the column edges, minimum
    // block size and all.
    let a = doc.append_element(
        Some(container),
        "div",
        Some("display:block;min-block-size:40px;break-inside:avoid"),
    );
    let b = doc.append_element(
        Some(container),
        "div",
        Some("display:block;min-block-size:40px;break-inside:avoid"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 300.0;
    page.height = 200.0;
    layout_single_page(&mut doc, &cascade, page).expect("layout Ok");

    // The second box does not fit below the first, and avoids a break
    // inside, so it starts the second column.
    let location = |id: usize| {
        let layout = doc.nodes[id].layout;
        (layout.location.x, layout.location.y, layout.size.height)
    };
    assert_eq!(location(a), (0.0, 0.0, 40.0));
    assert_eq!(location(b), (110.0, 0.0, 40.0));
    assert!(!doc.nodes[b].needs_relative_block_paint_offset);
}

#[test]
fn compute_multicol_layout_spaces_min_height_children_when_slicing_fails() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let container = doc.append_element(
        Some(body),
        "div",
        Some("display:block;column-count:2;column-gap:20px;width:200px;height:60px"),
    );
    let a = doc.append_element(
        Some(container),
        "div",
        Some("display:block;min-block-size:40px;break-inside:avoid"),
    );
    let b = doc.append_element(
        Some(container),
        "div",
        Some("display:block;min-block-size:40px;break-inside:avoid"),
    );
    // A rounded box taller than a column would be cut, and painting cannot
    // slice its corners, so the children are not sliced at all.
    doc.append_element(
        Some(container),
        "div",
        Some("display:block;height:70px;border:2px solid;border-radius:2px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 300.0;
    page.height = 200.0;
    layout_single_page(&mut doc, &cascade, page).expect("layout Ok");

    // The children keep the stacked approximation: `child_height + 2 *
    // |fragmentainer_height - child_height|` apart.
    let child_height = doc.nodes[a].layout.size.height;
    let step = child_height + 2.0 * (60.0_f32 - child_height).abs();
    assert_eq!(child_height, 40.0);
    assert_eq!(doc.nodes[a].layout.location.y, 0.0);
    assert_eq!(doc.nodes[b].layout.location.y, step);
    assert!(doc.nodes[b].needs_relative_block_paint_offset);
}

#[test]
fn manual_column_block_position_provenance_resets_on_relayout() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let container = doc.append_element(
        Some(body),
        "div",
        Some("display:block;column-count:2;width:200px;height:60px"),
    );
    let child = doc.append_element(
        Some(container),
        "div",
        Some("display:block;min-block-size:40px;break-inside:avoid;position:relative;top:12px"),
    );
    let rules = build_rule_tree(&doc);
    let mut cascade = cascade(&doc, &rules).expect("cascade");
    layout_single_page(&mut doc, &cascade, PageBox::A4).expect("column layout");
    assert!(doc.nodes[child].needs_relative_block_paint_offset());

    cascade.computed[container].column_count = ColumnCountValue::Auto;
    layout_single_page(&mut doc, &cascade, PageBox::A4).expect("ordinary block layout");
    assert!(!doc.nodes[child].needs_relative_block_paint_offset());
    assert_eq!(doc.nodes[child].layout.location.y, 12.0);
}

#[test]
fn balanced_auto_multicol_updates_its_container_fragment_height() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let container = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:100px;column-count:2;column-gap:0;background:green"),
    );
    for (height, extra) in [(25, ""), (50, ""), (25, ";break-before:avoid"), (50, "")] {
        doc.append_element(
            Some(container),
            "div",
            Some(&format!("display:block;height:{height}px{extra}")),
        );
    }

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 800.0;
    page.height = 600.0;
    layout_single_page(&mut doc, &cascade, page).expect("layout Ok");

    assert_eq!(doc.nodes[container].layout.size.height, 100.0);
    let fragment = doc
        .fragment_tree
        .fragments
        .iter()
        .find(|fragment| fragment.node_id == container)
        .expect("multicol container fragment");
    assert_eq!(fragment.rect.height, 100.0);
}

#[test]
fn compute_multicol_layout_clamps_auto_height_to_the_largest_min_constrained_child() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let container = doc.append_element(
        Some(body),
        "div",
        Some("display:block;column-count:2;column-gap:20px;width:200px"),
    );
    let a = doc.append_element(
        Some(container),
        "div",
        Some("display:block;min-block-size:40px;break-inside:avoid"),
    );
    let _b = doc.append_element(
        Some(container),
        "div",
        Some("display:block;min-block-size:40px;break-inside:avoid"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 300.0;
    page.height = 400.0;
    layout_single_page(&mut doc, &cascade, page).expect("layout Ok");

    // An ordinary auto-height block container would size to the *sum*
    // of its two 40px min-height children (80px). A multicol container
    // with min-constrained children instead clamps its own auto used
    // height to the largest child's used height: the children overflow
    // the box rather than growing it (see the comment above this
    // clamp in `compute_multicol_layout`).
    let child_height = doc.nodes[a].layout.size.height;
    assert!(child_height > 0.0, "child_height={child_height}");
    assert!(
        (doc.nodes[container].layout.size.height - child_height).abs() < 0.01,
        "container height={}, expected clamp to child height={child_height}",
        doc.nodes[container].layout.size.height,
    );
}

#[test]
fn multicol_definite_dimension_resolves_absolute_length_regardless_of_basis() {
    let doc = Document::new();
    assert_eq!(
        multicol_definite_dimension(&doc, Dimension::length(120.5), None),
        Some(120.5)
    );
    assert_eq!(
        multicol_definite_dimension(&doc, Dimension::length(120.5), Some(9999.0)),
        Some(120.5)
    );
}

#[test]
fn multicol_definite_dimension_resolves_percent_against_the_given_basis() {
    let doc = Document::new();
    assert_eq!(
        multicol_definite_dimension(&doc, Dimension::percent(0.25), Some(200.0)),
        Some(50.0)
    );
}

#[test]
fn multicol_definite_dimension_resolves_percent_against_a_zero_default_basis() {
    // `basis` defaults to 0.0 when the caller has no known containing
    // block size yet, so a percentage resolves to zero rather than
    // `None`.
    let doc = Document::new();
    assert_eq!(
        multicol_definite_dimension(&doc, Dimension::percent(0.5), None),
        Some(0.0)
    );
}

#[test]
fn multicol_definite_dimension_returns_none_for_intrinsic_sizing_keywords() {
    // Every non-definite keyword collapses to `None`: the caller falls
    // back to Taffy's ordinary intrinsic-sizing pass instead of a used
    // length.
    let doc = Document::new();
    for dimension in [
        Dimension::auto(),
        Dimension::min_content(),
        Dimension::max_content(),
        Dimension::fit_content(),
        Dimension::fit_content_px(40.0),
        Dimension::fit_content_percent(0.5),
        Dimension::stretch(),
        Dimension::content(),
    ] {
        assert_eq!(
            multicol_definite_dimension(&doc, dimension, Some(200.0)),
            None
        );
    }
}

#[test]
fn multicol_definite_dimension_clamps_a_negative_length_to_zero() {
    let doc = Document::new();
    assert_eq!(
        multicol_definite_dimension(&doc, Dimension::length(-25.0), None),
        Some(0.0)
    );
}

#[test]
fn multicol_definite_dimension_returns_none_for_a_non_finite_result() {
    // An infinite basis makes the resolved percentage non-finite; the
    // `is_finite()` guard turns that into `None` rather than an
    // infinite used size.
    let doc = Document::new();
    assert_eq!(
        multicol_definite_dimension(&doc, Dimension::percent(0.5), Some(f32::INFINITY)),
        None
    );
}

#[test]
fn relayout_nested_multicol_children_column_places_a_block_sibling_of_an_empty_direct_text_child() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    // An *auto*-height multicol container (unlike the definite-height
    // test above) routes every child through the auto-measurement pass
    // first (`auto_measurements` is `Some` here), which has its own
    // copy of the "reset a direct text child's style before measuring"
    // branch. An empty text node never gets a shaped layout, so it leaves
    // the container's auto height alone while the node still participates
    // in `relayout_nested_multicol_children`'s child list.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let container = doc.append_element(
        Some(body),
        "div",
        Some("display:block;column-count:2;column-gap:20px;width:200px"),
    );
    let a = doc.append_element(
        Some(container),
        "div",
        // A scroll container is monolithic, so it is not sliced into the
        // columns and keeps the strategy this test covers.
        Some("display:block;height:30px;overflow:hidden"),
    );
    let _text = doc.append_text(container, "");

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 300.0;
    page.height = 200.0;
    layout_single_page(&mut doc, &cascade, page).expect("layout Ok");

    // `a`'s width narrowing from the container's full 200px to one
    // 90px column ((200 - 20) / 2) is only possible through the custom
    // nested-fragmentation path -- the ordinary Taffy block algorithm
    // would stretch it to the full content width instead. That path
    // only runs by measuring every child in `children`, the empty
    // text node included, so this indirectly exercises its branch.
    assert!((doc.nodes[a].layout.size.width - 90.0).abs() < 0.01);
    assert!((doc.nodes[container].layout.size.height - 30.0).abs() < 0.01);
}

#[test]
fn vertical_fallback_multicol_preserves_logical_minimum_block_extent() {
    use raikiri_style::{build_rule_tree, cascade};

    for writing_mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"] {
        for (container_size, expected_height, second_y) in [
            ("", 40.0, 40.0),
            ("block-size:30px", 30.0, 60.0),
            ("block-size:60px", 60.0, 80.0),
        ] {
            let mut doc = Document::new();
            let html = doc.append_element(
                Some(0),
                "html",
                Some(format!("writing-mode:{writing_mode}")),
            );
            let body = doc.append_element(Some(html), "body", None::<&str>);
            let container = doc.append_element(
                Some(body),
                "div",
                Some(format!(
                    "display:block;column-count:3;inline-size:500px;{container_size}"
                )),
            );
            let children: Vec<_> = (0..2)
                .map(|_| {
                    doc.append_element(
                        Some(container),
                        "div",
                        Some("display:block;min-block-size:40px;break-inside:avoid"),
                    )
                })
                .collect();
            let rules = build_rule_tree(&doc);
            let cascade = cascade(&doc, &rules).expect("cascade Ok");
            let mut page = raikiri_traits::PageBox::new();
            page.width = 800.0;
            page.height = 600.0;
            layout_single_page(&mut doc, &cascade, page).expect("layout Ok");

            assert_eq!(doc.nodes[container].layout.size.height, expected_height);
            for &child in &children {
                assert_eq!(doc.nodes[child].layout.size.height, 40.0);
            }
            assert_eq!(doc.nodes[children[0]].layout.location.y, 0.0);
            assert_eq!(doc.nodes[children[1]].layout.location.y, second_y);
        }
    }
}

#[test]
fn vertical_fallback_preserves_explicit_physical_min_height() {
    use raikiri_style::{build_rule_tree, cascade};

    for minimum in [
        "min-block-size:40px;min-height:80px",
        "min-block-size:auto;min-height:80px",
        "min-block-size:calc(20px + 20px);min-height:calc(60px + 20px)",
        "min-block-size:40px;min-height:50%",
        "min-height:80px",
    ] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Some("writing-mode:vertical-rl"));
        let body = doc.append_element(Some(html), "body", None::<&str>);
        let parent = doc.append_element(
            Some(body),
            "div",
            Some("display:block;width:200px;height:160px;column-count:1"),
        );
        let child = doc.append_element(
            Some(parent),
            "div",
            Some(format!("display:block;{minimum}")),
        );
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade Ok");
        let mut page = raikiri_traits::PageBox::new();
        page.width = 800.0;
        page.height = 600.0;
        layout_single_page(&mut doc, &cascade, page).expect("layout Ok");
        assert_eq!(doc.nodes[child].layout.size.height, 80.0, "{minimum}");
    }
}

#[test]
fn relayout_nested_flex_float_records_column_local_geometry() {
    let (mut doc, multicol, second_float) = nested_flex_float_fixture("row", "horizontal-tb", 160);
    layout_nested_flex_float_fixture(&mut doc);

    let style = doc.nodes[multicol].multicol.expect("multicol style");
    let context = FragmentationContext::resolve(300.0, None, style).expect("fragment context");
    assert_eq!(context.column_count, 2);
    assert!((context.column_width - 142.0).abs() < 0.01);
    assert!((context.column_gap - 16.0).abs() < 0.01);

    let float_fragments: Vec<_> = doc
        .fragment_tree
        .fragments
        .iter()
        .filter(|fragment| fragment.node_id == second_float)
        .collect();
    assert!(
        !float_fragments.is_empty(),
        "the nested float needs a final fragment placement"
    );
    assert_eq!(float_fragments.len(), 1);
    assert_eq!(float_fragments[0].fragmentainer, 3);
    assert!((float_fragments[0].rect.y - 30.0).abs() < 0.01);
    let flex_item = doc.parent_of(second_float).expect("flex item parent");
    let tall_float = *doc.nodes[flex_item]
        .children
        .first()
        .expect("first float child");
    let flex_item_fragments: Vec<_> = doc
        .fragment_tree
        .fragments
        .iter()
        .enumerate()
        .filter(|(_, fragment)| fragment.node_id == flex_item)
        .collect();
    assert_eq!(flex_item_fragments.len(), 4);
    assert!(
        flex_item_fragments
            .iter()
            .enumerate()
            .all(|(index, (_, fragment))| {
                fragment.fragmentainer == index
                    && fragment.fragment_index == index
                    && fragment.fragment_count == 4
            })
    );
    let (last_column_item, item_fragment) = flex_item_fragments[3];
    assert_eq!(float_fragments[0].parent, Some(last_column_item));
    let item_clip = item_fragment
        .fragmentainer_clip
        .expect("flex item fragment clip");
    assert!((item_clip.x - context.column_offset_x(3)).abs() < 0.01);
    assert!((item_clip.width - context.column_width).abs() < 0.01);
    assert!((item_clip.height - 160.0).abs() < 0.01);
    let tall_fragment = doc
        .fragment_tree
        .fragments
        .iter()
        .find(|fragment| fragment.node_id == tall_float)
        .expect("first float fragment");
    assert_eq!(tall_fragment.fragmentainer, 0);
    assert_eq!(tall_fragment.parent, Some(flex_item_fragments[0].0));
    assert!(
        float_fragments
            .iter()
            .all(|fragment| (fragment.rect.y - 510.0).abs() > 0.01)
    );
    assert!(
        float_fragments
            .iter()
            .all(|fragment| fragment.fragmentainer_clip.is_some())
    );
    let clip = float_fragments[0]
        .fragmentainer_clip
        .expect("float fragment clip");
    assert!(clip.x.abs() < 0.01);
    assert!((clip.width - context.column_width).abs() < 0.01);
    assert!((clip.height - 160.0).abs() < 0.01);
    assert!(float_fragments.iter().enumerate().all(|(index, fragment)| {
        fragment.fragment_index == index && fragment.fragment_count == float_fragments.len()
    }));
}

#[test]
fn nested_flex_float_continues_text_past_declared_columns() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let multicol = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:200px;height:40px;column-count:2;column-gap:0"),
    );
    let flex = doc.append_element(
        Some(multicol),
        "div",
        Some("display:flex;flex-direction:row"),
    );
    let item = doc.append_element(Some(flex), "div", None::<&str>);
    doc.append_element(Some(item), "div", Some("float:left;width:1px;height:1px"));
    for _ in 0..12 {
        doc.append_text(item, "line");
        doc.append_element(Some(item), "br", None::<&str>);
    }

    layout_nested_flex_float_fixture(&mut doc);
    let lines = doc.nodes[item].ifc_lines().expect("item lines");
    assert!(
        lines.len() >= 12,
        "all source lines must survive fragmentation"
    );
    let ranges = doc.nodes[item]
        .ifc_multicol_fragments()
        .expect("fragmentainer ranges");
    assert!(
        ranges.iter().any(|range| range.fragmentainer >= 2),
        "text must continue beyond the two declared columns"
    );
}

#[test]
fn tiny_fragmentainers_bound_nested_float_fragments() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let multicol = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:200px;columns:2;max-height:0.001px"),
    );
    let flex = doc.append_element(
        Some(multicol),
        "div",
        Some("display:flex;flex-direction:row"),
    );
    let item = doc.append_element(
        Some(flex),
        "div",
        Some("font-size:0;line-height:0;margin-top:30px"),
    );
    let float = doc.append_element(Some(item), "div", Some("float:left;width:1px;height:2px"));
    doc.append_element(Some(item), "br", None::<&str>);

    layout_nested_flex_float_fixture(&mut doc);
    let count = doc
        .fragment_tree
        .fragments
        .iter()
        .filter(|fragment| fragment.node_id == float)
        .count();
    assert!(count > 1, "float must cross more than one column");
    assert!(count <= 1_024, "float expansion must remain bounded");
    let last_float = doc
        .fragment_tree
        .fragments
        .iter()
        .filter(|fragment| fragment.node_id == float)
        .max_by_key(|fragment| fragment.fragmentainer)
        .expect("last float fragment");
    let last_clip = last_float.fragmentainer_clip.expect("last float clip");
    assert!(
        last_float.fragmentainer as f32 * 0.001 + last_clip.height >= 32.0,
        "the final float fragment must paint the remaining height"
    );
    let mut parent = last_float.parent;
    while let Some(parent_id) = parent {
        let ancestor = &doc.fragment_tree.fragments[parent_id];
        if let Some(clip) = ancestor.fragmentainer_clip {
            assert!(
                clip.height >= last_clip.height,
                "ancestor clips must include the float overflow"
            );
        }
        parent = ancestor.parent;
    }
}

#[test]
fn tiny_fragmentainers_keep_every_line_in_a_fragment() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let multicol = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:200px;columns:2;max-height:0.001px"),
    );
    let flex = doc.append_element(
        Some(multicol),
        "div",
        Some("display:flex;flex-direction:row"),
    );
    let item = doc.append_element(Some(flex), "div", None::<&str>);
    doc.append_element(Some(item), "div", Some("float:left;width:1px;height:2px"));
    for _ in 0..12 {
        doc.append_text(item, "line");
        doc.append_element(Some(item), "br", None::<&str>);
    }

    layout_nested_flex_float_fixture(&mut doc);
    let lines = doc.nodes[item].ifc_lines().expect("item lines");
    // One line per source line: the trailing `<br>` ends the paragraph
    // without an empty line after it.
    assert_eq!(lines.len(), 12, "all source lines must survive");
    let ranges = doc.nodes[item]
        .ifc_multicol_fragments()
        .expect("fragmentainer ranges");
    let last_range = ranges.last().expect("last line range");
    assert_eq!(last_range.line_end, lines.len());
    // Every fragmentainer that holds a line has a fragment of the item.
    for range in ranges {
        assert!(
            doc.fragment_tree.fragments.iter().any(|fragment| {
                fragment.node_id == item && fragment.fragmentainer == range.fragmentainer
            }),
            "fragmentainer {} has no fragment",
            range.fragmentainer
        );
    }
}

#[test]
fn direct_nested_text_root_records_each_column_range() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let multicol = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:200px;columns:2;column-gap:0;max-height:40px"),
    );
    let paragraph = doc.append_element(
        Some(multicol),
        "p",
        Some("display:block;margin:0;font-size:10px;line-height:10px"),
    );
    for _ in 0..8 {
        doc.append_text(paragraph, "line");
        doc.append_element(Some(paragraph), "br", None::<&str>);
    }

    layout_nested_flex_float_fixture(&mut doc);
    let lines = doc.nodes[paragraph].ifc_lines().expect("paragraph lines");
    let fragments: Vec<_> = doc
        .fragment_tree
        .fragments
        .iter()
        .filter(|fragment| fragment.node_id == paragraph && fragment.line_start.is_some())
        .collect();
    assert!(fragments.len() >= 2, "text must span columns");
    assert_eq!(
        fragments.last().expect("last fragment").line_end,
        Some(lines.len())
    );
}

#[test]
fn multicol_block_children_with_lines_get_text_ranges() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let multicol = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:200px;columns:2;column-gap:0;height:20px"),
    );
    let first = doc.append_element(
        Some(multicol),
        "div",
        Some("display:block;height:30px;font-size:10px;line-height:10px"),
    );
    doc.append_text(first, "one");
    doc.append_element(Some(first), "br", None::<&str>);
    let second = doc.append_element(
        Some(multicol),
        "div",
        Some("display:block;font-size:10px;line-height:10px"),
    );
    doc.append_text(second, "two");
    doc.append_element(Some(second), "br", None::<&str>);

    layout_nested_flex_float_fixture(&mut doc);
    assert_ne!(doc.nodes[multicol].layout_display(), Display::Flex);
    assert!(doc.nodes[first].is_ifc_root(), "first child must own lines");
    assert!(
        doc.nodes[first].ifc_multicol_fragments().is_some(),
        "column text ranges must be assigned"
    );
}

#[test]
fn reused_fragment_clip_expands_to_later_child_overflow() {
    let mut fragment = crate::fragment::LayoutFragment {
        node_id: 0,
        parent: None,
        fragmentainer: 0,
        rect: crate::fragment::FragmentRect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 1.0,
        },
        fragmentainer_clip: Some(crate::fragment::FragmentRect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 1.0,
        }),
        fragment_index: 0,
        fragment_count: 1,
        line_start: None,
        line_end: None,
    };
    extend_fragment_clip(&mut fragment, 2.0);
    assert_eq!(fragment.fragmentainer_clip.expect("clip").height, 2.0);
    extend_fragment_clip(&mut fragment, 0.5);
    assert_eq!(fragment.fragmentainer_clip.expect("clip").height, 2.0);
}

#[test]
fn relayout_nested_flex_float_records_only_committed_positions() {
    let (mut doc, _, second_float) = nested_flex_float_fixture("row", "horizontal-tb", 160);
    layout_nested_flex_float_fixture(&mut doc);

    let float_fragments: Vec<_> = doc
        .fragment_tree
        .fragments
        .iter()
        .filter(|fragment| fragment.node_id == second_float)
        .collect();
    assert_eq!(
        float_fragments.len(),
        1,
        "the second float is committed once"
    );
    assert!(
        (float_fragments[0].rect.y - 510.0).abs() > 0.01,
        "the rejected intermediate float position must not be retained"
    );
    assert_eq!(float_fragments[0].fragment_index, 0);
    assert_eq!(float_fragments[0].fragment_count, 1);
}

#[test]
fn nested_float_fragment_budget_returns_a_layout_error() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::{LayoutError, PageBox};

    let (mut doc, _, _) = nested_flex_float_fixture("row", "horizontal-tb", 1);
    doc.fragment_tree.limit = 8;
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 800.0;
    page.height = 600.0;

    assert!(matches!(
        layout_single_page(&mut doc, &cascade, page),
        Err(LayoutError::FragmentLimitExceeded { limit: 8 })
    ));
    assert_eq!(doc.fragment_tree.fragments.len(), 8);
}

#[test]
fn zero_fragment_budget_rejects_the_first_multicol_fragment() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::{LayoutError, PageBox};

    let (mut doc, _, _) = nested_flex_float_fixture("row", "horizontal-tb", 160);
    doc.fragment_tree.limit = 0;
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 800.0;
    page.height = 600.0;

    assert!(matches!(
        layout_single_page(&mut doc, &cascade, page),
        Err(LayoutError::FragmentLimitExceeded { limit: 0 })
    ));
    assert!(doc.fragment_tree.fragments.is_empty());
}

#[test]
fn constrained_chain_budget_rejects_before_installing_partial_origins() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::{LayoutError, PageBox};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;font:20px/20px sans-serif"),
    );
    let mc = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:100px;height:20px;column-count:2;column-gap:20px;orphans:1;widows:1"),
    );
    let wrapper = doc.append_element(Some(mc), "div", Some("display:block;width:80px"));
    let paragraph = doc.append_element(Some(wrapper), "p", Some("display:block;margin:0"));
    for (index, letter) in ["A", "B", "C", "D"].into_iter().enumerate() {
        if index > 0 {
            doc.append_element(Some(paragraph), "br", None::<&str>);
        }
        doc.append_text(paragraph, letter);
    }
    doc.fragment_tree.limit = 6;
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).unwrap();
    let mut page = PageBox::new();
    page.width = 180.0;
    page.height = 160.0;
    assert!(matches!(
        layout_single_page(&mut doc, &cascade, page),
        Err(LayoutError::FragmentLimitExceeded { limit: 6 })
    ));
    assert!(doc.fragment_tree.fragments.is_empty());
    assert!(
        !doc.nodes[paragraph]
            .ifc
            .as_ref()
            .unwrap()
            .multicol_fragment_origins_recorded
    );
}

#[test]
fn nested_multicol_child_stops_when_the_shared_fragment_budget_is_exhausted() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::{LayoutError, PageBox};

    let mut saw_limit_error = false;
    for limit in [1, 2, 3, 4, 5, 6, 8, 12, 16, 24, 32, 64] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", None::<&str>);
        let body = doc.append_element(Some(html), "body", None::<&str>);
        let outer = doc.append_element(
            Some(body),
            "div",
            Some("display:block;width:300px;columns:2;max-height:80px"),
        );
        let inner = doc.append_element(
            Some(outer),
            "div",
            Some("display:block;width:200px;columns:2;max-height:80px"),
        );
        let flex = doc.append_element(Some(inner), "div", Some("display:flex;flex-direction:row"));
        let item = doc.append_element(Some(flex), "div", None::<&str>);
        doc.append_element(
            Some(item),
            "div",
            Some("float:left;width:100px;height:500px"),
        );
        doc.fragment_tree.limit = limit;
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade Ok");
        let mut page = PageBox::new();
        page.width = 800.0;
        page.height = 600.0;

        let result = layout_single_page(&mut doc, &cascade, page);
        if matches!(result, Err(LayoutError::FragmentLimitExceeded { .. })) {
            saw_limit_error = true;
            assert_eq!(doc.fragment_tree.fragments.len(), limit);
        } else {
            assert!(result.is_ok(), "{result:?}");
            assert!(doc.fragment_tree.fragments.len() <= limit);
        }
    }
    assert!(saw_limit_error);
}

#[test]
fn nested_float_fragment_budget_accumulates_across_items() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::{LayoutError, PageBox};

    let (mut doc, multicol, _) = nested_flex_float_fixture("row", "horizontal-tb", 160);
    let body = doc.parent_of(multicol).expect("body");
    let mut siblings = Vec::new();
    for _ in 0..8 {
        let sibling = doc.append_element(
            Some(body),
            "div",
            Some("display:block;width:300px;columns:100px auto;max-height:160px"),
        );
        siblings.push(sibling);
        let flex = doc.append_element(
            Some(sibling),
            "div",
            Some("display:flex;flex-direction:row"),
        );
        let item = doc.append_element(Some(flex), "div", None::<&str>);
        doc.append_element(
            Some(item),
            "div",
            Some("float:left;width:100px;height:500px"),
        );
    }
    doc.fragment_tree.limit = 16;
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 800.0;
    page.height = 600.0;

    let result = layout_single_page(&mut doc, &cascade, page);
    assert!(
        matches!(
            result,
            Err(LayoutError::FragmentLimitExceeded { limit: 16 })
        ),
        "result: {result:?}, fragments: {}",
        doc.fragment_tree.fragments.len()
    );
    assert_eq!(doc.fragment_tree.fragments.len(), 16);
    assert!(
        doc.fragment_tree
            .fragments
            .iter()
            .any(|fragment| siblings.contains(&fragment.node_id))
    );
    let source_count = doc
        .fragment_tree
        .fragments
        .iter()
        .map(|fragment| fragment.node_id)
        .collect::<std::collections::HashSet<_>>()
        .len();
    assert!(
        source_count > 2,
        "fragments must span multiple source nodes"
    );
}

#[test]
fn relayout_nested_flex_float_replaces_old_fragment_records() {
    let (mut doc, multicol, second_float) = nested_flex_float_fixture("row", "horizontal-tb", 160);
    layout_nested_flex_float_fixture(&mut doc);
    let first: Vec<_> = doc
        .fragment_tree
        .fragments
        .iter()
        .filter(|fragment| fragment.node_id == second_float)
        .copied()
        .collect();
    assert!(!first.is_empty(), "the first layout records the float");

    doc.set_element_inline_style(
        multicol,
        Some(
            "display:block;width:300px;columns:100px auto;max-height:120px;border:3px solid pink"
                .into(),
        ),
    );
    layout_nested_flex_float_fixture(&mut doc);
    let second: Vec<_> = doc
        .fragment_tree
        .fragments
        .iter()
        .filter(|fragment| fragment.node_id == second_float)
        .copied()
        .collect();

    assert!(!second.is_empty(), "the second layout records the float");
    assert_ne!(first, second, "the second pass replaces old placements");
}

#[test]
fn relayout_nested_flex_float_scope_is_row_only() {
    let (mut row_doc, _, row_float) = nested_flex_float_fixture("row", "horizontal-tb", 160);
    layout_nested_flex_float_fixture(&mut row_doc);
    assert!(
        row_doc
            .fragment_tree
            .fragments
            .iter()
            .any(|fragment| fragment.node_id == row_float)
    );

    for (direction, writing_mode) in [("column", "horizontal-tb"), ("row", "vertical-rl")] {
        let (mut doc, multicol, second_float) =
            nested_flex_float_fixture(direction, writing_mode, 160);
        layout_nested_flex_float_fixture(&mut doc);
        if writing_mode != "horizontal-tb" {
            assert_eq!(
                doc.nodes[multicol].authored_writing_mode,
                Some(raikiri_style::property::WritingMode::VerticalRl)
            );
        }
        assert!(
            !doc.fragment_tree
                .fragments
                .iter()
                .any(|fragment| fragment.node_id == second_float),
            "float fragments should be scoped out for flex-direction:{direction}, writing-mode:{writing_mode}"
        );
    }
}

fn nested_flex_float_fixture(
    flex_direction: &str,
    writing_mode: &str,
    max_height: u16,
) -> (Document, usize, usize) {
    let mut doc = Document::new();
    let html_style =
        (writing_mode != "horizontal-tb").then(|| format!("writing-mode:{writing_mode}"));
    let html = doc.append_element(Some(0), "html", html_style.as_deref());
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let multicol_writing_mode = if writing_mode == "horizontal-tb" {
        String::new()
    } else {
        format!(";writing-mode:{writing_mode}")
    };
    let multicol = doc.append_element(
        Some(body),
        "div",
        Some(format!(
            "display:block;width:300px;columns:100px auto;max-height:{max_height}px;border:3px solid pink{multicol_writing_mode}"
        )),
    );
    let flex = doc.append_element(
        Some(multicol),
        "div",
        Some(format!("display:flex;flex-direction:{flex_direction}")),
    );
    let flex_item = doc.append_element(
        Some(flex),
        "div",
        Some("border:4px solid teal;outline:4px solid blue"),
    );
    doc.append_element(
        Some(flex_item),
        "div",
        Some("float:left;border:3px solid black;height:500px;width:100px;background:yellow"),
    );
    doc.append_element(Some(flex_item), "br", None::<&str>);
    let second_float = doc.append_element(
        Some(flex_item),
        "div",
        Some("float:left;background:cyan;width:100px"),
    );
    doc.append_element(
        Some(second_float),
        "div",
        Some("height:30px;width:30px;background:purple;display:inline-block"),
    );
    (doc, multicol, second_float)
}

fn layout_nested_flex_float_fixture(doc: &mut Document) {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let rules = build_rule_tree(doc);
    let cascade = cascade(doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 800.0;
    page.height = 600.0;
    layout_single_page(doc, &cascade, page).expect("layout Ok");
}

#[test]
fn max_height_multicol_fragment_does_not_expand_to_float_overflow() {
    let (mut doc, multicol, _) = nested_flex_float_fixture("row", "horizontal-tb", 160);
    layout_nested_flex_float_fixture(&mut doc);

    let fragment = doc
        .fragment_tree
        .fragments
        .iter()
        .find(|fragment| fragment.node_id == multicol)
        .expect("multicol fragment");
    assert!(
        fragment.rect.height <= 166.0,
        "max-height:160px plus 3px borders must bound the fragment, got {}",
        fragment.rect.height
    );
}

fn break_flow_positions(edge: &str, before: bool, height: u32) -> Vec<(f32, f32, f32)> {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    let columns = doc.append_element(
        Some(body),
        "div",
        Some(&format!(
            "display:block;columns:2;column-fill:auto;column-gap:0;width:100px;height:{height}px"
        )),
    );
    let mut boxes = Vec::new();
    for index in 0..4 {
        let edge = if index == if before { 3 } else { 2 } {
            edge
        } else {
            ""
        };
        boxes.push(doc.append_element(
            Some(columns),
            "div",
            Some(&format!(
                "display:block;height:50px;break-inside:avoid;{edge}"
            )),
        ));
    }
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
    let mut page = raikiri_traits::PageBox::new();
    page.width = 800.0;
    page.height = 600.0;
    super::super::layout_single_page(&mut doc, &cascade, page).unwrap();
    boxes
        .into_iter()
        .map(|id| {
            let layout = doc.nodes[id].layout;
            (layout.location.x, layout.location.y, layout.size.width)
        })
        .collect()
}

#[test]
fn before_avoid_moves_a_connected_run_between_columns() {
    assert_eq!(
        break_flow_positions("break-before:avoid", true, 160),
        vec![
            (0.0, 0.0, 50.0),
            (0.0, 50.0, 50.0),
            (50.0, 0.0, 50.0),
            (50.0, 50.0, 50.0)
        ]
    );
}

#[test]
fn after_avoid_moves_a_connected_run_between_columns() {
    assert_eq!(
        break_flow_positions("break-after:avoid", false, 160),
        vec![
            (0.0, 0.0, 50.0),
            (0.0, 50.0, 50.0),
            (50.0, 0.0, 50.0),
            (50.0, 50.0, 50.0)
        ]
    );
}

#[test]
fn column_specific_avoid_moves_a_connected_run_between_columns() {
    assert_eq!(
        break_flow_positions("break-before:avoid-column", true, 160),
        vec![
            (0.0, 0.0, 50.0),
            (0.0, 50.0, 50.0),
            (50.0, 0.0, 50.0),
            (50.0, 50.0, 50.0)
        ]
    );
}

#[test]
fn always_forces_the_innermost_column_without_a_page_break() {
    assert_eq!(
        break_flow_positions("break-after:always", false, 250),
        vec![
            (0.0, 0.0, 50.0),
            (0.0, 50.0, 50.0),
            (0.0, 100.0, 50.0),
            (50.0, 0.0, 50.0)
        ]
    );
}

#[test]
fn column_forces_a_boundary_even_when_the_current_column_has_room() {
    assert_eq!(
        break_flow_positions("break-after:column", false, 250),
        vec![
            (0.0, 0.0, 50.0),
            (0.0, 50.0, 50.0),
            (0.0, 100.0, 50.0),
            (50.0, 0.0, 50.0)
        ]
    );
}

fn lineless_fixture(container_css: &str, children_css: &[&str]) -> (Document, usize, Vec<usize>) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let parent = doc.append_element(Some(body), "div", Some(container_css));
    let children = children_css
        .iter()
        .map(|css| doc.append_element(Some(parent), "div", Some(*css)))
        .collect();
    layout_nested_flex_float_fixture(&mut doc);
    (doc, parent, children)
}

fn wrapped_lineless_fixture(wrapper_css: &str) -> (Document, usize, usize) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let parent = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:100px;columns:4;column-gap:0"),
    );
    let wrapper_style = format!("display:block;{wrapper_css}");
    let wrapper = doc.append_element(Some(parent), "div", Some(&wrapper_style));
    let child = doc.append_element(
        Some(wrapper),
        "div",
        Some("display:block;width:25px;height:120px"),
    );
    layout_nested_flex_float_fixture(&mut doc);
    (doc, parent, child)
}

fn nested_wrapped_lineless_fixture() -> (Document, usize, usize) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let parent = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:100px;columns:4;column-gap:0"),
    );
    let wrapper = doc.append_element(Some(parent), "div", Some("display:flow-root"));
    let spanner = doc.append_element(Some(wrapper), "div", Some("display:block;column-span:all"));
    let child = doc.append_element(
        Some(spanner),
        "div",
        Some("display:block;width:25px;height:400px"),
    );
    layout_nested_flex_float_fixture(&mut doc);
    (doc, parent, child)
}

fn fragment_rects(doc: &Document, node: usize) -> Vec<(usize, f32, f32, f32)> {
    doc.fragment_tree
        .fragments
        .iter()
        .filter(|fragment| fragment.node_id == node)
        .map(|fragment| {
            (
                fragment.fragmentainer,
                fragment.rect.x,
                fragment.rect.y,
                fragment.rect.height,
            )
        })
        .collect()
}

#[test]
fn a_block_without_lines_is_sliced_into_definite_height_columns() {
    let (doc, _, children) = lineless_fixture(
        "display:block;width:100px;height:100px;columns:4;column-gap:0;column-fill:auto",
        &["display:block;height:400px"],
    );
    assert_eq!(
        fragment_rects(&doc, children[0]),
        vec![
            (0, 0.0, 0.0, 100.0),
            (1, 25.0, 0.0, 100.0),
            (2, 50.0, 0.0, 100.0),
            (3, 75.0, 0.0, 100.0)
        ]
    );
}

#[test]
fn blocks_without_lines_are_balanced_and_keep_break_avoidance() {
    let (doc, parent, children) = lineless_fixture(
        "display:block;width:200px;columns:2;column-gap:0",
        &["display:block;height:150px", "display:block;height:50px"],
    );
    // 200px of content balances into two 100px columns.
    assert_eq!(doc.nodes[parent].layout.size.height, 100.0);
    assert_eq!(
        fragment_rects(&doc, children[0]),
        vec![(0, 0.0, 0.0, 100.0), (1, 100.0, 0.0, 50.0)]
    );
    assert_eq!(
        fragment_rects(&doc, children[1]),
        vec![(1, 100.0, 50.0, 50.0)]
    );

    let (doc, parent, children) = lineless_fixture(
        "display:block;width:300px;columns:3;column-gap:0",
        &[
            "display:block;height:40px;break-inside:avoid",
            "display:block;height:40px;break-inside:avoid",
        ],
    );
    assert_eq!(doc.nodes[parent].layout.size.height, 40.0);
    assert_eq!(
        fragment_rects(&doc, children[1]),
        vec![(1, 100.0, 0.0, 40.0)]
    );
}

#[test]
fn auto_height_columns_balance_lineless_blocks_inside_wrappers() {
    let (doc, parent, children) = lineless_fixture(
        "display:block;width:100px;columns:4;column-gap:0",
        &["display:block;width:25px;height:120px"],
    );
    assert_eq!(doc.nodes[parent].layout.size.height, 30.0);
    assert_eq!(
        fragment_rects(&doc, children[0]),
        vec![
            (0, 0.0, 0.0, 30.0),
            (1, 25.0, 0.0, 30.0),
            (2, 50.0, 0.0, 30.0),
            (3, 75.0, 0.0, 30.0),
        ]
    );

    for wrapper_css in ["", "display:flow-root", "padding:2px 0"] {
        let (doc, parent, child) = wrapped_lineless_fixture(wrapper_css);
        let height = doc.nodes[parent].layout.size.height;
        assert!(
            height <= 34.0,
            "wrapper {wrapper_css:?} should balance to about one quarter of the block, got {height}"
        );
        let fragments = fragment_rects(&doc, child);
        assert_eq!(
            fragments
                .iter()
                .map(|fragment| fragment.0)
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3],
            "wrapper {wrapper_css:?} should split its child across all four columns: {fragments:?}"
        );
        assert_eq!(
            fragments.iter().map(|fragment| fragment.3).sum::<f32>(),
            120.0,
            "wrapper {wrapper_css:?} should preserve the child's full block size"
        );
    }

    for wrapper_css in [
        "break-inside:avoid",
        "padding:2px 0;box-decoration-break:clone",
    ] {
        let (doc, parent, child) = wrapped_lineless_fixture(wrapper_css);
        assert!(
            doc.nodes[parent].layout.size.height >= 120.0,
            "wrapper {wrapper_css:?} should keep its full block size"
        );
        assert!(
            fragment_rects(&doc, child).len() <= 1,
            "wrapper {wrapper_css:?} should stay monolithic"
        );
    }
}

#[test]
fn auto_height_columns_balance_wrappers_with_padding_taller_than_a_column() {
    let (doc, parent, child) = wrapped_lineless_fixture("padding-top:100px");

    assert_eq!(doc.nodes[parent].layout.size.height, 55.0);
    assert_eq!(
        fragment_rects(&doc, child)
            .iter()
            .map(|fragment| fragment.3)
            .sum::<f32>(),
        120.0
    );
}

#[test]
fn auto_height_columns_balance_lineless_blocks_inside_nested_wrappers() {
    let (mut doc, parent, child) = nested_wrapped_lineless_fixture();
    assert_eq!(doc.nodes[parent].layout.size.height, 100.0);
    assert!(
        with_bridged_session(&mut doc, |session| lineless::supported_with_wrappers(
            session, parent
        ))
        .is_some(),
        "a span below flow-root is regular column content"
    );
    let fragments = fragment_rects(&doc, child);
    assert_eq!(
        fragments
            .iter()
            .map(|fragment| fragment.0)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3],
        "nested wrappers should not make the child monolithic: {fragments:?}"
    );
    assert_eq!(
        fragments.iter().map(|fragment| fragment.3).sum::<f32>(),
        400.0
    );
}

#[test]
fn auto_balance_preserves_spanners_in_the_multicol_formatting_context() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let parent = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:100px;columns:4;column-gap:0"),
    );
    let wrapper = doc.append_element(Some(parent), "div", Some("display:block"));
    let spanner = doc.append_element(Some(wrapper), "div", Some("display:block;column-span:all"));
    doc.append_element(
        Some(spanner),
        "div",
        Some("display:block;width:25px;height:400px"),
    );
    layout_nested_flex_float_fixture(&mut doc);

    assert!(
        with_bridged_session(&mut doc, |session| lineless::supported_with_wrappers(
            session, parent
        ))
        .is_none(),
        "a spanner in the multicol formatting context must keep the spanning path active"
    );
}

#[test]
fn wrappers_keep_empty_spanners_out_of_the_stream() {
    let spanners = |wrapper_css: &str| {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", None::<&str>);
        let body = doc.append_element(Some(html), "body", Some("margin:0"));
        let parent = doc.append_element(
            Some(body),
            "div",
            Some("display:block;width:100px;columns:3"),
        );
        let wrapper = doc.append_element(Some(parent), "div", Some(wrapper_css));
        for css in [
            "display:block;column-span:all;height:10px;margin-bottom:50px",
            "display:block;column-span:all;height:10px;margin-top:80px",
        ] {
            doc.append_element(Some(wrapper), "div", Some(css));
        }
        layout_nested_flex_float_fixture(&mut doc);
        (doc, parent)
    };

    // The margins of the two spanners collapse into the larger one. The
    // plain wrapper makes no box, so the spanners are among the children
    // that the container places.
    let (mut doc, parent) = spanners("display:block");
    assert!(
        doc.children_through_contents(parent)
            .into_iter()
            .all(
                |child| with_bridged_session(&mut doc, |session| spanning::is_spanner(
                    session, child
                ))
            )
    );
    assert_eq!(doc.nodes[parent].layout.size.height, 100.0);

    // Below a flow root they are ordinary blocks of the columns.
    let (mut doc, parent) = spanners("display:flow-root");
    assert!(
        with_bridged_session(&mut doc, |session| lineless::supported_with_wrappers(
            session, parent
        ))
        .is_some()
    );
}

#[test]
fn auto_balance_applies_wrapper_edge_trimming_only_to_boundary_leaves() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let parent = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:100px;columns:2;column-gap:0"),
    );
    let outer = doc.append_element(Some(parent), "div", Some("display:block;padding-top:2px"));
    let inner = doc.append_element(Some(outer), "div", Some("display:flow-root"));
    let children: Vec<_> = (0..2)
        .map(|_| {
            doc.append_element(
                Some(inner),
                "div",
                Some("display:block;width:25px;height:20px"),
            )
        })
        .collect();
    layout_nested_flex_float_fixture(&mut doc);

    for child in children {
        assert_eq!(
            fragment_rects(&doc, child)
                .iter()
                .map(|fragment| fragment.3)
                .sum::<f32>(),
            20.0,
            "wrapper padding must not trim a non-boundary leaf"
        );
    }
}

#[test]
fn containers_with_content_or_replaced_children_keep_other_strategies() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let parent = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:100px;columns:2"),
    );
    let child = doc.append_element(Some(parent), "div", Some("display:block;height:100px"));
    layout_nested_flex_float_fixture(&mut doc);
    assert!(
        with_bridged_session(&mut doc, |session| lineless::supported(
            session, parent, true
        ))
        .is_some()
    );
    doc.append_text(child, "text");
    assert!(
        with_bridged_session(&mut doc, |session| lineless::supported(
            session, parent, true
        ))
        .is_none()
    );

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let parent = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:100px;columns:2"),
    );
    doc.append_element(Some(parent), "input", Some("display:block;height:100px"));
    layout_nested_flex_float_fixture(&mut doc);
    assert!(
        with_bridged_session(&mut doc, |session| lineless::supported(
            session, parent, true
        ))
        .is_none()
    );
}

#[test]
fn only_a_relative_leaf_block_keeps_its_insets_in_the_stream() {
    let supported = |child: &str| {
        let (mut doc, parent, _) = lineless_fixture(
            "display:block;width:100px;height:100px;columns:2;column-gap:0;column-fill:auto",
            &[child],
        );
        with_bridged_session(&mut doc, |session| {
            lineless::supported(session, parent, true)
        })
        .is_some()
    };
    assert!(supported(
        "display:block;height:50px;position:relative;left:5px"
    ));
    assert!(supported(
        "display:block;height:50px;position:relative;left:5px;right:-5px"
    ));
    // The painter adds the offset of a relative box only.
    assert!(!supported(
        "display:block;height:50px;position:sticky;left:5px"
    ));
    // A block-axis offset moves the box across pages, which pagination
    // measures from the location the columns gave it.
    assert!(!supported(
        "display:block;height:50px;position:relative;top:5px"
    ));
    assert!(!supported(
        "display:block;height:50px;position:relative;bottom:5px"
    ));
}

#[test]
fn a_relative_containing_block_with_insets_keeps_other_strategies() {
    let supported = |block: &str| {
        let (mut doc, parent, _) = absolute_fixture(
            DEFINITE_COLUMNS,
            &[(block, &["position:absolute;top:0;width:10px;height:20px"])],
        );
        with_bridged_session(&mut doc, |session| {
            lineless::supported(session, parent, true)
        })
        .is_some()
    };
    assert!(supported("display:block;position:relative;height:50px"));
    assert!(!supported(
        "display:block;position:relative;left:5px;height:50px"
    ));
}

#[test]
fn an_orthogonal_block_without_lines_is_not_sliced() {
    let (doc, _, children) = lineless_fixture(
        "display:block;width:100px;height:100px;columns:4;column-gap:0;column-fill:auto",
        &["display:block;height:400px;writing-mode:vertical-rl"],
    );
    assert!(fragment_rects(&doc, children[0]).len() <= 1);
}

#[test]
fn a_block_with_size_containment_is_not_sliced() {
    // Size containment makes the box monolithic (CSS Contain 2, 3.1).
    let (mut doc, parent, _) = lineless_fixture(
        "display:block;width:100px;height:100px;columns:4;column-gap:0;column-fill:auto",
        &["display:block;height:400px;contain:size"],
    );
    assert!(
        with_bridged_session(&mut doc, |session| lineless::supported(
            session, parent, true
        ))
        .is_none()
    );
}

#[test]
fn a_nested_multicol_slices_a_block_without_lines_into_its_columns() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let outer = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:200px;height:100px;columns:2;column-gap:0;column-fill:auto"),
    );
    let inner = doc.append_element(
        Some(outer),
        "div",
        Some("display:block;height:100px;columns:2;column-gap:0;column-fill:auto"),
    );
    let child = doc.append_element(Some(inner), "div", Some("display:block;height:200px"));
    layout_nested_flex_float_fixture(&mut doc);
    let pieces = fragment_rects(&doc, child);
    assert_eq!(
        pieces
            .iter()
            .map(|&(_, x, y, height)| (x, y, height))
            .collect::<Vec<_>>(),
        vec![(0.0, 0.0, 100.0), (50.0, 0.0, 100.0)]
    );
}

#[test]
fn a_nested_multicol_taller_than_the_outer_column_continues_in_the_next_one() {
    // 500px of nested columns cut at the end of a 300px outer column end
    // with a 200px row in the outer overflow column, whose own overflow
    // column takes the rest of the box (CSS Multi-column 1, 8.2).
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let outer = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:200px;height:300px;column-count:1"),
    );
    let inner = doc.append_element(
        Some(outer),
        "div",
        Some("display:block;height:500px;column-count:1"),
    );
    let child = doc.append_element(Some(inner), "div", Some("display:block;height:600px"));
    layout_nested_flex_float_fixture(&mut doc);
    assert_eq!(
        fragment_rects(&doc, child),
        vec![
            (0, 0.0, 0.0, 300.0),
            (1, 0.0, 0.0, 200.0),
            (2, 216.0, 0.0, 100.0)
        ]
    );
    assert_eq!(doc.nodes[inner].multicol_rows.len(), 2);
    // Layout numbers the records of the container once it finishes, so the
    // later row's box is not a first fragment.
    let records: Vec<_> = doc
        .fragment_tree
        .fragments
        .iter()
        .filter(|fragment| fragment.node_id == inner)
        .collect();
    assert_eq!(
        records
            .iter()
            .filter(|fragment| fragment.fragment_index == 0)
            .count(),
        1
    );
    assert!(records.iter().all(|fragment| fragment.fragmentainer == 0
        || fragment.fragment_index > 0 && fragment.fragment_count == records.len()));
}

/// A 300px outer column holding a nested column container with `inner_css`,
/// whose 500px of content is cut at the end of the outer column. Returns the
/// container, its child, and the border-box heights of the container's boxes
/// in the outer columns, from the first.
fn continued_nested_multicol(inner_css: &str) -> (Document, usize, usize, Vec<f32>) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let outer = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:200px;height:300px;column-count:1"),
    );
    let inner = doc.append_element(
        Some(outer),
        "div",
        Some(&format!(
            "display:block;height:500px;column-count:1;padding:10px 0;{inner_css}"
        )),
    );
    let child = doc.append_element(Some(inner), "div", Some("display:block;height:500px"));
    layout_nested_flex_float_fixture(&mut doc);
    // The container's own record numbers its first box 0 of 3 and holds the
    // outer column's share, so the later records are the boxes of the rows.
    let heights = doc
        .fragment_tree
        .fragments
        .iter()
        .filter(|fragment| fragment.node_id == inner && fragment.fragment_index > 0)
        .map(|fragment| fragment.rect.height)
        .collect();
    (doc, inner, child, heights)
}

#[test]
fn a_cloned_nested_multicol_keeps_its_padding_in_every_outer_column() {
    // A box that slices holds content up to the end of each outer column and
    // has no padding in the rows after the first.
    let (doc, _, child, boxes) = continued_nested_multicol("");
    assert_eq!(
        fragment_rects(&doc, child),
        vec![(0, 0.0, 10.0, 290.0), (1, 0.0, 0.0, 210.0)]
    );
    assert_eq!(boxes, vec![300.0, 220.0]);

    // A box that clones has the padding in both rows, so each holds 20px
    // less content and the content starts below the padding.
    let (doc, _, child, boxes) = continued_nested_multicol("box-decoration-break:clone");
    assert_eq!(
        fragment_rects(&doc, child),
        vec![(0, 0.0, 10.0, 280.0), (1, 0.0, 10.0, 220.0)]
    );
    assert_eq!(boxes, vec![300.0, 240.0]);
}

#[test]
fn a_cloned_nested_multicol_fills_its_rows_with_paragraph_lines_between_its_padding() {
    // Twenty 10px lines in a container with 10px of padding on each side
    // fill 80px of a 100px outer column, so the rows hold 8, 8 and 4 lines.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let outer = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:300px;height:100px;columns:3;column-gap:0;column-fill:auto"),
    );
    let inner = doc.append_element(
        Some(outer),
        "div",
        Some(
            "display:block;columns:1;padding:10px 0;box-decoration-break:clone;\
             font-size:10px;line-height:10px",
        ),
    );
    let paragraph = doc.append_element(Some(inner), "div", Some("display:block"));
    for _ in 0..20 {
        doc.append_text(paragraph, "a");
        doc.append_element(Some(paragraph), "br", None::<&str>);
    }
    layout_nested_flex_float_fixture(&mut doc);
    let ranges: Vec<_> = doc.nodes[paragraph]
        .ifc_multicol_fragments()
        .expect("column ranges")
        .iter()
        .map(|range| (range.line_start, range.line_end, range.fragmentainer))
        .collect();
    assert_eq!(ranges, [(0, 8, 0), (8, 16, 1), (16, 20, 2)]);
    let heights: Vec<_> = doc
        .fragment_tree
        .fragments
        .iter()
        .filter(|fragment| fragment.node_id == inner && fragment.fragment_index > 0)
        .map(|fragment| fragment.rect.height)
        .collect();
    assert_eq!(heights, [100.0, 100.0, 60.0]);
    assert_eq!(doc.nodes[inner].multicol_rows.len(), 3);
}

/// A column container of `columns` columns, 100px high and filled in order,
/// holding a one-column container of `lines` 10px lines styled by
/// `inner_css`, not yet laid out. Returns the document, the inner container
/// and its paragraph.
fn nested_paragraph_document(
    columns: usize,
    inner_css: &str,
    lines: usize,
) -> (Document, usize, usize) {
    styled_nested_paragraph_document(columns, 0.0, inner_css, "", lines)
}

/// `nested_paragraph_document` with `paragraph_css` on the paragraph, after a
/// block of `lead` px when that is not zero.
fn styled_nested_paragraph_document(
    columns: usize,
    lead: f32,
    inner_css: &str,
    paragraph_css: &str,
    lines: usize,
) -> (Document, usize, usize) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let outer = doc.append_element(
        Some(body),
        "div",
        Some(&format!(
            "display:block;width:{}px;height:100px;columns:{columns};column-gap:0;\
             column-fill:auto",
            columns * 100
        )),
    );
    if lead > 0.0 {
        doc.append_element(
            Some(outer),
            "div",
            Some(&format!("display:block;height:{lead}px")),
        );
    }
    let inner = doc.append_element(
        Some(outer),
        "div",
        Some(&format!(
            "display:block;columns:1;font-size:10px;line-height:10px;{inner_css}"
        )),
    );
    let paragraph = doc.append_element(
        Some(inner),
        "div",
        Some(&format!("display:block;{paragraph_css}")),
    );
    for _ in 0..lines {
        doc.append_text(paragraph, "a");
        doc.append_element(Some(paragraph), "br", None::<&str>);
    }
    (doc, inner, paragraph)
}

/// `nested_paragraph_document` laid out. Returns the document, the inner
/// container and the paragraph's column ranges as (line start, line end,
/// fragmentainer).
fn nested_paragraph_rows(
    columns: usize,
    inner_css: &str,
    lines: usize,
) -> (Document, usize, Vec<(usize, usize, usize)>) {
    let (mut doc, inner, paragraph) = nested_paragraph_document(columns, inner_css, lines);
    layout_nested_flex_float_fixture(&mut doc);
    let ranges = doc.nodes[paragraph]
        .ifc_multicol_fragments()
        .map(|ranges| {
            ranges
                .iter()
                .map(|range| (range.line_start, range.line_end, range.fragmentainer))
                .collect()
        })
        .unwrap_or_default();
    (doc, inner, ranges)
}

#[test]
fn the_paragraph_rows_of_a_nested_multicol_continue_in_outer_overflow_columns() {
    // Three rows of a 100px column fit the content, but the outer container
    // declares two columns, so the third row is in an overflow column.
    let (doc, inner, ranges) = nested_paragraph_rows(2, "", 25);
    assert_eq!(ranges, [(0, 10, 0), (10, 20, 1), (20, 25, 2)]);
    let heights: Vec<_> = doc
        .fragment_tree
        .fragments
        .iter()
        .filter(|fragment| fragment.node_id == inner && fragment.fragment_index > 0)
        .map(|fragment| fragment.rect.height)
        .collect();
    assert_eq!(heights, [100.0, 100.0, 50.0]);
    assert_eq!(doc.nodes[inner].multicol_rows.len(), 3);

    // A box that clones its padding has less room in each row, which can
    // need one more outer column than the lines alone.
    let (doc, inner, ranges) =
        nested_paragraph_rows(2, "padding:10px 0;box-decoration-break:clone", 20);
    assert_eq!(ranges, [(0, 8, 0), (8, 16, 1), (16, 20, 2)]);
    assert_eq!(doc.nodes[inner].multicol_rows.len(), 3);
}

/// Lay out `styled_nested_paragraph_document` after a block of `lead` px under a
/// fragment budget of `limit`.
fn nested_paragraph_with_limit(
    columns: usize,
    lead: f32,
    inner_css: &str,
    lines: usize,
    limit: usize,
) -> (Document, usize, Result<(), raikiri_traits::LayoutError>) {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let (mut doc, inner, _) = styled_nested_paragraph_document(columns, lead, inner_css, "", lines);
    doc.fragment_tree.limit = limit;
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 800.0;
    page.height = 600.0;
    let result = layout_single_page(&mut doc, &cascade, page).map(|_| ());
    (doc, inner, result)
}

#[test]
fn paragraph_rows_the_fragment_budget_cannot_hold_report_exhaustion() {
    use raikiri_traits::LayoutError;

    // Whatever the budget, the layout either holds the three rows or reports
    // that it ran out; it never drops the rows and lays the lines out in one
    // column.
    let mut exhausted = false;
    for limit in 1..40 {
        let (doc, inner, result) = nested_paragraph_with_limit(3, 0.0, "", 25, limit);
        match result {
            Ok(()) => assert_eq!(
                doc.nodes[inner].multicol_rows.len(),
                3,
                "limit {limit} dropped the rows"
            ),
            Err(LayoutError::FragmentLimitExceeded { .. }) => exhausted = true,
            Err(other) => panic!("limit {limit}: {other:?}"),
        }
    }
    assert!(exhausted);
}

#[test]
fn a_nested_multicol_without_room_or_lines_for_paragraph_rows_does_not_continue() {
    // Padding that a clone repeats in every row leaves them no room for
    // content in a 100px column.
    let (doc, inner, ranges) =
        nested_paragraph_rows(2, "padding:60px 0;box-decoration-break:clone", 25);
    assert!(doc.nodes[inner].multicol_rows.is_empty());
    assert!(
        ranges
            .iter()
            .all(|&(_, _, fragmentainer)| fragmentainer == 0)
    );

    // A container that is taller than the column only for the margin of an
    // empty block has no paragraph to fill rows with.
    let (mut doc, inner, _) = styled_nested_paragraph_document(2, 0.0, "", "margin-top:150px", 0);
    layout_nested_flex_float_fixture(&mut doc);
    assert!(doc.nodes[inner].multicol_rows.is_empty());
}

#[test]
fn a_nested_multicol_that_cannot_afford_its_rows_here_may_start_in_the_next_column() {
    // The container follows a 60px block, so its rows from here (4 + 10 + 10 +
    // 1 lines) need more records than a budget can have left, while from the
    // top of the next column (10 + 10 + 5 lines) they need fewer. Running out
    // while planning from here does not end the layout if that plan works.
    let rows: Vec<Option<usize>> = (1..40)
        .map(|limit| {
            let (doc, inner, result) = nested_paragraph_with_limit(3, 60.0, "", 25, limit);
            result.ok().map(|()| doc.nodes[inner].multicol_rows.len())
        })
        .collect();
    assert!(rows.contains(&None), "some budget is too small");
    assert!(
        rows.contains(&Some(3)),
        "a tight budget moves the container"
    );
    assert_eq!(rows.last(), Some(&Some(4)), "a large one keeps it here");
}

#[test]
fn a_nested_multicol_that_fits_the_next_column_whole_moves_there_when_its_rows_do_not_fit() {
    // Eight lines after a 60px block continue in a second row from here, but
    // fit a column whole. A budget too small for the two rows moves the
    // container instead of failing, and a larger one never fails after a
    // smaller one succeeded.
    let rows: Vec<Option<usize>> = (1..16)
        .map(|limit| {
            let (doc, inner, result) = nested_paragraph_with_limit(3, 60.0, "", 8, limit);
            result.ok().map(|()| doc.nodes[inner].multicol_rows.len())
        })
        .collect();
    let first_ok = rows
        .iter()
        .position(Option::is_some)
        .expect("some budget works");
    assert!(rows[first_ok..].iter().all(Option::is_some), "{rows:?}");
    assert_eq!(rows[first_ok], Some(0), "the container moves whole");
    assert_eq!(rows.last(), Some(&Some(2)), "a large budget keeps the rows");
}

#[test]
fn paragraph_rows_planning_stops_at_the_fragment_budget() {
    use raikiri_traits::LayoutError;

    // Content that needs far more rows than the budget has records for is
    // reported as exhausted without the rows being made.
    let (doc, inner, result) = nested_paragraph_with_limit(2, 0.0, "", 3000, 20);
    assert!(matches!(
        result,
        Err(LayoutError::FragmentLimitExceeded { .. })
    ));
    assert!(doc.nodes[inner].multicol_rows.is_empty());

    // So are the fragments of a single row: ten inner columns cut the lines of
    // the first row into ten fragments, more than the budget has records for
    // after the container's own layout.
    let (doc, inner, result) =
        nested_paragraph_with_limit(2, 0.0, "columns:10;column-gap:0", 3000, 30);
    assert!(matches!(
        result,
        Err(LayoutError::FragmentLimitExceeded { .. })
    ));
    assert!(doc.nodes[inner].multicol_rows.is_empty());
}

#[test]
fn the_rows_of_a_cloned_nested_multicol_start_their_column_rules_below_the_padding() {
    let (doc, inner, ..) = continued_nested_multicol("box-decoration-break:clone");
    let rows = &doc.nodes[inner].multicol_rows;
    assert_eq!(
        rows.iter()
            .map(|row| (row.context.origin_y, row.height))
            .collect::<Vec<_>>(),
        vec![(10.0, 280.0), (10.0, 220.0)]
    );
    // Every row's box is a whole box, so none is a slice of one.
    assert!(rows.iter().all(|row| row.slice.is_none()));

    let (doc, inner, ..) = continued_nested_multicol("");
    let slices: Vec<_> = doc.nodes[inner]
        .multicol_rows
        .iter()
        .map(|row| row.slice)
        .collect();
    assert_eq!(slices, [Some((0.0, 300.0)), Some((300.0, 520.0))]);
}

#[test]
fn a_cloned_nested_multicol_whose_padding_leaves_a_row_no_room_is_not_continued_in_rows() {
    // The padding above and below fills the 300px outer column, so a row has
    // no room for content. A box that slices gives up only its first row's
    // padding, and continues in three.
    let (doc, inner, ..) = continued_nested_multicol("padding:150px 0;box-decoration-break:clone");
    assert!(doc.nodes[inner].multicol_rows.is_empty());
    let (doc, inner, ..) = continued_nested_multicol("padding:150px 0");
    assert_eq!(doc.nodes[inner].multicol_rows.len(), 3);
}

#[test]
fn content_past_the_last_column_of_a_definite_height_multicol_overflows_inline() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let outer = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:200px;height:100px;column-count:1"),
    );
    doc.append_element(Some(outer), "div", Some("display:block;height:100px"));
    let inner = doc.append_element(
        Some(outer),
        "div",
        Some("display:block;height:50px;column-count:1"),
    );
    doc.append_element(Some(inner), "div", Some("display:block;height:50px"));
    layout_nested_flex_float_fixture(&mut doc);
    assert_eq!(doc.nodes[inner].layout.location, Point { x: 216.0, y: 0.0 });
}

#[test]
fn a_nested_multicol_moves_to_the_next_outer_column_rather_than_cutting_a_box() {
    // The break before the nested container is preferred to cutting its box
    // at the end of the first outer column (CSS Fragmentation 3, 4.4).
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let outer = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:200px;height:100px;columns:2;column-gap:0;column-fill:auto"),
    );
    doc.append_element(Some(outer), "div", Some("display:block;height:60px"));
    let inner = doc.append_element(
        Some(outer),
        "div",
        Some("display:block;columns:2;column-gap:0"),
    );
    doc.append_element(Some(inner), "div", Some("display:block;height:100px"));
    layout_nested_flex_float_fixture(&mut doc);
    assert_eq!(doc.nodes[inner].layout.location, Point { x: 100.0, y: 0.0 });
    assert!(doc.nodes[inner].multicol_rows.is_empty());

    // A break avoided before the container leaves cutting the box instead.
    doc.set_element_inline_style(
        doc.nodes[outer].children[0],
        Some("display:block;height:60px;break-after:avoid".into()),
    );
    layout_nested_flex_float_fixture(&mut doc);
    assert_eq!(doc.nodes[inner].layout.location.x, 0.0);
    assert_eq!(doc.nodes[inner].multicol_rows.len(), 2);
}

#[test]
fn the_trailing_margin_of_blocks_without_lines_stays_in_the_container() {
    let (doc, parent, _) = lineless_fixture(
        "display:block;width:100px;columns:1",
        &["display:block;height:40px;margin-bottom:20px"],
    );
    assert_eq!(doc.nodes[parent].layout.size.height, 60.0);
}

#[test]
fn blocks_with_block_edge_decorations_slice_them_as_the_whole_box_moved_up() {
    for decoration in ["border-top:2px solid", "padding-bottom:2px"] {
        let (doc, _, children) = lineless_fixture(
            "display:block;width:100px;height:100px;columns:2;column-gap:0;column-fill:auto",
            &[&format!("display:block;height:150px;{decoration}")],
        );
        // Each piece is the whole box, moved up by what the pieces before it
        // took, so the column clip shows only the part that belongs to it.
        assert_eq!(
            fragment_rects(&doc, children[0]),
            vec![(0, 0.0, 0.0, 152.0), (1, 50.0, -100.0, 152.0)],
            "{decoration}"
        );
    }
    // A decoration that repeats at each break, or an effect sized from the
    // whole box, cannot be sliced, so the box stays whole.
    for decoration in [
        "border-top:2px solid;box-decoration-break:clone",
        "border-top:2px solid;border-radius:2px",
    ] {
        let (doc, _, children) = lineless_fixture(
            "display:block;width:100px;height:100px;columns:2;column-gap:0;column-fill:auto",
            &[&format!("display:block;height:200px;{decoration}")],
        );
        assert!(fragment_rects(&doc, children[0]).len() <= 1, "{decoration}");
    }
    let (doc, _, children) = lineless_fixture(
        "display:block;width:100px;height:100px;columns:2;column-gap:0;column-fill:auto",
        &["display:block;height:200px;border-left:2px solid"],
    );
    assert_eq!(fragment_rects(&doc, children[0]).len(), 2);
}

#[test]
fn block_end_padding_taller_than_a_column_does_not_stall_the_columns() {
    let (doc, _, children) = lineless_fixture(
        "display:block;width:300px;height:100px;columns:3;column-gap:0;column-fill:auto",
        &["display:block;height:1px;padding-bottom:101px"],
    );
    assert_eq!(
        fragment_rects(&doc, children[0]),
        vec![
            (0, 0.0, 0.0, 102.0),
            (1, 100.0, -1.0, 102.0),
            (2, 200.0, -101.0, 102.0)
        ]
    );
}

#[test]
fn the_padding_of_a_block_in_a_wrapper_resolves_against_the_wrapper() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let parent = doc.append_element(Some(body), "div", Some(DEFINITE_COLUMNS));
    let wrapper = doc.append_element(Some(parent), "div", Some("display:block;padding:0 20px"));
    let leaf = doc.append_element(
        Some(wrapper),
        "div",
        Some("display:block;height:105px;padding-bottom:50%"),
    );
    layout_nested_flex_float_fixture(&mut doc);
    // The 50% is 30px of the wrapper's 60px content box, so the 135px block
    // breaks at the column end 5px above its padding. Resolved against the
    // 100px column it would be 50px, and the break would fall 15px early.
    let rects = fragment_rects(&doc, leaf);
    assert_eq!(
        rects
            .iter()
            .map(|rect| (rect.0, rect.2))
            .collect::<Vec<_>>(),
        vec![(0, 0.0), (1, -100.0)]
    );
}

#[test]
fn the_last_block_of_a_padded_wrapper_moves_with_the_padding_that_would_break_away() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let parent = doc.append_element(Some(body), "div", Some(DEFINITE_COLUMNS));
    let wrapper = doc.append_element(
        Some(parent),
        "div",
        Some("display:block;padding-bottom:50px"),
    );
    let first = doc.append_element(Some(wrapper), "div", Some("display:block;height:50px"));
    let last = doc.append_element(Some(wrapper), "div", Some("display:block;height:50px"));
    layout_nested_flex_float_fixture(&mut doc);
    // The second block fills the rest of the first column and the padding
    // does not fit, so the block follows the padding to the second column
    // instead of leaving it alone there.
    assert_eq!(fragment_rects(&doc, first), vec![(0, 0.0, 0.0, 50.0)]);
    assert_eq!(fragment_rects(&doc, last), vec![(1, 0.0, 0.0, 50.0)]);
    // The wrapper's second piece shows the last 100px of its 150px, the
    // block and the padding, not the 50px of padding that the gap above the
    // block would leave.
    assert_eq!(
        fragment_rects(&doc, wrapper),
        vec![(0, 0.0, 0.0, 150.0), (1, 100.0, -50.0, 150.0)]
    );
}

#[test]
fn the_padding_of_a_wrapper_that_fills_a_column_is_part_of_the_box_it_shows() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let parent = doc.append_element(Some(body), "div", Some(DEFINITE_COLUMNS));
    let wrapper = doc.append_element(
        Some(parent),
        "div",
        Some("display:block;padding-bottom:100px;border-bottom:2px solid"),
    );
    let block = doc.append_element(Some(wrapper), "div", Some("display:block;height:50px"));
    layout_nested_flex_float_fixture(&mut doc);
    // The padding runs from the end of the block to the end of the first
    // column and on into the second, so the second piece is moved up by the
    // whole 100px the first one shows, not by the 50px of the block.
    assert_eq!(fragment_rects(&doc, block)[0], (0, 0.0, 0.0, 50.0));
    assert_eq!(
        fragment_rects(&doc, wrapper),
        vec![(0, 0.0, 0.0, 152.0), (1, 100.0, -100.0, 152.0)]
    );
}

#[test]
fn the_margin_a_column_break_cuts_is_still_part_of_the_box_a_wrapper_shows() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let parent = doc.append_element(Some(body), "div", Some(DEFINITE_COLUMNS));
    let wrapper = doc.append_element(
        Some(parent),
        "div",
        Some("display:block;padding-bottom:48px;border-bottom:2px solid"),
    );
    doc.append_element(Some(wrapper), "div", Some("display:block;height:40px"));
    let last = doc.append_element(
        Some(wrapper),
        "div",
        Some("display:block;height:50px;margin-top:10px"),
    );
    layout_nested_flex_float_fixture(&mut doc);
    // The last block fills the rest of the first column and its padding and
    // border do not fit, so it follows them to the second column, where the
    // 10px margin above it is cut. The wrapper is 150px tall with the margin,
    // so its second piece is moved up by the 40px of the first block and the
    // margin, and the border ends the column.
    assert_eq!(fragment_rects(&doc, last)[0], (1, 0.0, 0.0, 50.0));
    assert_eq!(
        fragment_rects(&doc, wrapper),
        vec![(0, 0.0, 0.0, 150.0), (1, 100.0, -50.0, 150.0)]
    );

    // Blocks with no block size between the two let their margins collapse
    // with it.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let parent = doc.append_element(Some(body), "div", Some(DEFINITE_COLUMNS));
    let wrapper = doc.append_element(
        Some(parent),
        "div",
        Some("display:block;padding-bottom:48px;border-bottom:2px solid"),
    );
    doc.append_element(Some(wrapper), "div", Some("display:block;height:40px"));
    doc.append_element(Some(wrapper), "div", Some("display:block;margin-top:10px"));
    doc.append_element(
        Some(wrapper),
        "div",
        Some("display:block;margin-bottom:5px"),
    );
    doc.append_element(Some(wrapper), "div", Some("display:block;height:50px"));
    layout_nested_flex_float_fixture(&mut doc);
    assert_eq!(
        fragment_rects(&doc, wrapper),
        vec![(0, 0.0, 0.0, 150.0), (1, 100.0, -50.0, 150.0)]
    );

    // A block that fits the column exactly leaves the margin to the next
    // block as well.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let parent = doc.append_element(Some(body), "div", Some(DEFINITE_COLUMNS));
    let wrapper = doc.append_element(
        Some(parent),
        "div",
        Some("display:block;padding-bottom:10px"),
    );
    doc.append_element(Some(wrapper), "div", Some("display:block;height:100px"));
    let second = doc.append_element(
        Some(wrapper),
        "div",
        Some("display:block;height:80px;margin-top:10px"),
    );
    layout_nested_flex_float_fixture(&mut doc);
    assert_eq!(fragment_rects(&doc, second)[0], (1, 0.0, 0.0, 80.0));
    assert_eq!(
        fragment_rects(&doc, wrapper),
        vec![(0, 0.0, 0.0, 200.0), (1, 100.0, -110.0, 200.0)]
    );
}

#[test]
fn an_absolute_box_in_a_sliced_block_stays_in_its_column() {
    let (doc, _, ids) = absolute_fixture(
        DEFINITE_COLUMNS,
        &[(
            "display:block;position:relative;height:150px;border-top:5px solid;border-bottom:5px solid",
            &["position:absolute;top:130px;width:10px;height:10px"],
        )],
    );
    let absolute = ids[0].1[0];
    let fragments = &doc.fragment_tree.fragments;
    let in_column: Vec<_> = fragments
        .iter()
        .filter(|fragment| fragment.node_id == absolute)
        .map(|fragment| {
            // The parent piece is the whole block moved up by the 100px the
            // first piece took; the box and its clip move up with it.
            let parent = &fragments[fragment.parent.unwrap()];
            let clip = fragment.fragmentainer_clip.unwrap();
            (
                fragment.fragmentainer,
                parent.rect.y + fragment.rect.y,
                parent.rect.y + clip.y,
            )
        })
        .collect();
    // 135px down the block is 35px down the second column.
    assert_eq!(in_column, vec![(1, 35.0, 0.0)]);
}

#[test]
fn auto_filled_blocks_without_lines_report_the_fragment_budget() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::{LayoutError, PageBox};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let parent = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:100px;columns:2;column-fill:auto"),
    );
    for _ in 0..16 {
        doc.append_element(Some(parent), "div", Some("display:block;height:10px"));
    }
    doc.fragment_tree.limit = 8;
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    assert!(matches!(
        layout_single_page(&mut doc, &cascade, PageBox::A4),
        Err(LayoutError::FragmentLimitExceeded { limit: 8 })
    ));
}

#[test]
fn blocks_painting_whole_box_effects_are_not_sliced() {
    for effect in [
        "background-image:linear-gradient(red,blue)",
        "border-radius:4px",
        "box-shadow:0 0 2px black",
        "outline:1px solid",
        "border-image:linear-gradient(red,blue) 1;border-left:2px solid",
        "opacity:0.5",
        "mix-blend-mode:multiply",
    ] {
        let (doc, _, children) = lineless_fixture(
            "display:block;width:100px;height:100px;columns:2;column-gap:0;column-fill:auto",
            &[&format!("display:block;height:200px;{effect}")],
        );
        assert!(fragment_rects(&doc, children[0]).len() <= 1, "{effect}");
    }
    let (doc, _, children) = lineless_fixture(
        "display:block;width:100px;height:100px;columns:2;column-gap:0;column-fill:auto",
        &["display:block;height:200px;background-color:red"],
    );
    assert_eq!(fragment_rects(&doc, children[0]).len(), 2);
    // Boxes that each fit in a column are not cut, so they keep their effects.
    let (doc, _, children) = lineless_fixture(
        "display:block;width:100px;height:100px;columns:2;column-gap:0;column-fill:auto",
        &[
            "display:block;height:100px;box-shadow:0 0 2px black",
            "display:block;height:100px;box-shadow:0 0 2px black",
        ],
    );
    assert_eq!(
        fragment_rects(&doc, children[1]),
        vec![(1, 50.0, 0.0, 100.0)]
    );
}

#[test]
fn a_minimum_height_makes_columns_of_blocks_without_lines_taller() {
    let (doc, parent, children) = lineless_fixture(
        "display:block;width:200px;columns:2;column-gap:0;min-height:200px",
        &["display:block;height:200px"],
    );
    assert_eq!(doc.nodes[parent].layout.size.height, 200.0);
    assert_eq!(
        fragment_rects(&doc, children[0]),
        vec![(0, 0.0, 0.0, 200.0)]
    );
}

#[test]
fn a_minimum_height_makes_definite_columns_of_blocks_without_lines_taller() {
    let (doc, parent, children) = lineless_fixture(
        "display:block;width:200px;height:100px;min-height:200px;columns:2;column-gap:0;column-fill:auto",
        &["display:block;height:200px"],
    );
    assert_eq!(doc.nodes[parent].layout.size.height, 200.0);
    assert_eq!(
        fragment_rects(&doc, children[0]),
        vec![(0, 0.0, 0.0, 200.0)]
    );
}

#[test]
fn an_inherited_vertical_writing_mode_keeps_blocks_without_lines_whole() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(
        Some(html),
        "body",
        Some("margin:0;writing-mode:vertical-rl"),
    );
    let parent = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:100px;height:100px;columns:2;column-gap:0;column-fill:auto"),
    );
    let child = doc.append_element(Some(parent), "div", Some("display:block;height:200px"));
    layout_nested_flex_float_fixture(&mut doc);
    assert!(
        with_bridged_session(&mut doc, |session| lineless::supported(
            session, parent, true
        ))
        .is_none()
    );
    assert!(fragment_rects(&doc, child).len() <= 1);
}

#[test]
fn blocks_without_lines_fill_the_used_height_of_the_container() {
    // A maximum below the authored height limits the columns.
    let (doc, parent, children) = lineless_fixture(
        "display:block;width:100px;height:200px;max-height:100px;columns:2;column-gap:0;column-fill:auto",
        &["display:block;height:150px"],
    );
    assert_eq!(doc.nodes[parent].layout.size.height, 100.0);
    assert_eq!(
        fragment_rects(&doc, children[0]),
        vec![(0, 0.0, 0.0, 100.0), (1, 50.0, 0.0, 50.0)]
    );
    // A flex item stretched by its container keeps the stretched height.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let flex = doc.append_element(Some(body), "div", Some("display:flex;height:200px"));
    let parent = doc.append_element(
        Some(flex),
        "div",
        Some("display:block;width:100px;columns:2;column-gap:0"),
    );
    let child = doc.append_element(Some(parent), "div", Some("display:block;height:200px"));
    layout_nested_flex_float_fixture(&mut doc);
    assert_eq!(doc.nodes[parent].layout.size.height, 200.0);
    assert_eq!(fragment_rects(&doc, child), vec![(0, 0.0, 0.0, 200.0)]);
}

#[test]
fn a_decorated_block_too_long_for_the_budget_falls_back_whole() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let parent = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:100px;height:1px;columns:2;column-gap:0;column-fill:auto"),
    );
    let child = doc.append_element(
        Some(parent),
        "div",
        Some("display:block;height:100000px;background-image:linear-gradient(red,blue)"),
    );
    doc.fragment_tree.limit = 64;
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 800.0;
    page.height = 600.0;
    layout_single_page(&mut doc, &cascade, page).expect("layout Ok");
    assert!(fragment_rects(&doc, child).len() <= 1);
}

#[test]
fn a_decorated_block_that_stays_whole_does_not_hide_an_exhausted_budget() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::{LayoutError, PageBox};

    const DECORATED: &str = "display:block;height:1px;background-image:linear-gradient(red,blue)";
    for children in [
        [DECORATED, "display:block;height:100000px"],
        // The budget runs out on the decorated block, which is not cut.
        ["display:block;height:63px", DECORATED],
    ] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", None::<&str>);
        let body = doc.append_element(Some(html), "body", Some("margin:0"));
        let parent = doc.append_element(
            Some(body),
            "div",
            Some("display:block;width:100px;height:1px;columns:2;column-gap:0;column-fill:auto"),
        );
        for child in children {
            doc.append_element(Some(parent), "div", Some(child));
        }
        doc.fragment_tree.limit = 64;
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade Ok");
        let mut page = PageBox::new();
        page.width = 800.0;
        page.height = 600.0;
        assert!(
            matches!(
                layout_single_page(&mut doc, &cascade, page),
                Err(LayoutError::FragmentLimitExceeded { limit: 64 })
            ),
            "{children:?}"
        );
    }
}

#[test]
fn a_grid_stretched_multicol_keeps_its_area_height() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let grid = doc.append_element(
        Some(body),
        "div",
        Some("display:grid;grid-template-rows:200px;grid-template-columns:100px"),
    );
    let parent = doc.append_element(
        Some(grid),
        "div",
        Some("display:block;columns:2;column-gap:0"),
    );
    let child = doc.append_element(Some(parent), "div", Some("display:block;height:200px"));
    layout_nested_flex_float_fixture(&mut doc);
    assert_eq!(doc.nodes[parent].layout.size.height, 200.0);
    assert_eq!(fragment_rects(&doc, child), vec![(0, 0.0, 0.0, 200.0)]);
}

#[test]
fn an_inset_positioned_multicol_keeps_its_containing_block_height() {
    let (doc, parent, children) = lineless_fixture(
        "display:block;position:absolute;top:0;bottom:0;width:100px;columns:2;column-gap:0",
        &["display:block;height:200px"],
    );
    assert_eq!(doc.nodes[parent].layout.size.height, 600.0);
    assert_eq!(
        fragment_rects(&doc, children[0]),
        vec![(0, 0.0, 0.0, 200.0)]
    );
}

#[test]
fn a_maximum_height_only_bounds_balanced_columns_of_blocks_without_lines() {
    // A maximum that does not bind leaves the columns balanced.
    let (doc, parent, children) = lineless_fixture(
        "display:block;width:100px;max-height:300px;columns:2;column-gap:0",
        &["display:block;height:200px"],
    );
    assert_eq!(doc.nodes[parent].layout.size.height, 100.0);
    assert_eq!(
        fragment_rects(&doc, children[0]),
        vec![(0, 0.0, 0.0, 100.0), (1, 50.0, 0.0, 100.0)]
    );
    // A binding maximum shortens them.
    let (doc, parent, children) = lineless_fixture(
        "display:block;width:100px;max-height:50px;columns:2;column-gap:0",
        &["display:block;height:200px"],
    );
    assert_eq!(doc.nodes[parent].layout.size.height, 50.0);
    assert_eq!(fragment_rects(&doc, children[0])[0], (0, 0.0, 0.0, 50.0));
}

#[test]
fn a_column_flex_item_keeps_its_flexed_height() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let flex = doc.append_element(
        Some(body),
        "div",
        Some("display:flex;flex-direction:column;height:200px"),
    );
    let parent = doc.append_element(
        Some(flex),
        "div",
        Some("display:block;flex:1;width:100px;columns:2;column-gap:0"),
    );
    let child = doc.append_element(Some(parent), "div", Some("display:block;height:200px"));
    layout_nested_flex_float_fixture(&mut doc);
    assert_eq!(doc.nodes[parent].layout.size.height, 200.0);
    assert_eq!(fragment_rects(&doc, child), vec![(0, 0.0, 0.0, 200.0)]);
}

#[test]
fn a_shrunk_column_flex_item_keeps_its_flexed_height() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let flex = doc.append_element(
        Some(body),
        "div",
        Some("display:flex;flex-direction:column;height:100px"),
    );
    let parent = doc.append_element(
        Some(flex),
        "div",
        Some("display:block;min-height:0;width:100px;columns:2;column-gap:0"),
    );
    let child = doc.append_element(Some(parent), "div", Some("display:block;height:300px"));
    layout_nested_flex_float_fixture(&mut doc);
    assert_eq!(doc.nodes[parent].layout.size.height, 100.0);
    assert_eq!(
        fragment_rects(&doc, child),
        vec![
            (0, 0.0, 0.0, 100.0),
            (1, 50.0, 0.0, 100.0),
            (2, 100.0, 0.0, 100.0)
        ]
    );
}

#[test]
fn a_shrunk_column_flex_item_bounds_its_columns_inside_its_padding() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let flex = doc.append_element(
        Some(body),
        "div",
        Some("display:flex;flex-direction:column;height:100px"),
    );
    // The looser maximum height leaves the flexed height in charge.
    let parent = doc.append_element(
        Some(flex),
        "div",
        Some(
            "display:block;min-height:0;max-height:90px;padding:10px 0;width:100px;\
             columns:2;column-gap:0",
        ),
    );
    let child = doc.append_element(Some(parent), "div", Some("display:block;height:300px"));
    layout_nested_flex_float_fixture(&mut doc);
    assert_eq!(doc.nodes[parent].layout.size.height, 100.0);
    let heights: Vec<f32> = fragment_rects(&doc, child)
        .into_iter()
        .map(|(_, _, _, height)| height)
        .collect();
    assert_eq!(heights, vec![80.0, 80.0, 80.0, 60.0]);
}

#[test]
fn blocks_without_lines_skip_children_that_generate_no_box() {
    let lineless_children = |children: &[&str], text: Option<&str>| {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", None::<&str>);
        let body = doc.append_element(Some(html), "body", Some("margin:0"));
        let parent = doc.append_element(
            Some(body),
            "div",
            Some("display:block;width:100px;columns:2;column-gap:0"),
        );
        doc.append_comment(Some(parent), "skipped");
        if let Some(text) = text {
            doc.append_text(parent, text);
        }
        let ids: Vec<usize> = children
            .iter()
            .map(|style| doc.append_element(Some(parent), "div", Some(*style)))
            .collect();
        layout_nested_flex_float_fixture(&mut doc);
        (
            with_bridged_session(&mut doc, |session| {
                super::lineless::supported(session, parent, true)
            }),
            ids,
        )
    };
    // A box with no display and a comment are skipped, and a minimum height
    // makes the other block one without lines.
    let (found, ids) = lineless_children(&["display:none", "display:block;min-height:10px"], None);
    assert_eq!(found, Some(vec![ids[1]]));
    // An absolutely positioned child is placed outside the column stream.
    let (found, _) = lineless_children(
        &[
            "display:block;height:10px",
            "display:block;position:absolute;height:10px",
        ],
        None,
    );
    assert_eq!(found, None);
    // Text that renders keeps the other strategies.
    let (found, _) = lineless_children(&["display:block;height:10px"], Some("text"));
    assert_eq!(found, None);
}

#[test]
fn a_zero_height_multicol_keeps_its_blocks_without_lines_unsliced() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let parent = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:100px;height:0;columns:2;column-gap:0"),
    );
    let child = doc.append_element(Some(parent), "div", Some("display:block;height:10px"));
    layout_nested_flex_float_fixture(&mut doc);
    assert_eq!(doc.nodes[parent].layout.size.height, 0.0);
    assert!(fragment_rects(&doc, child).len() <= 1);
}

#[test]
fn compute_multicol_layout_keeps_foundational_child_layouts_when_slicing_fails() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let container = doc.append_element(
        Some(body),
        "div",
        Some("display:block;column-count:2;column-gap:20px;width:200px;height:60px"),
    );
    doc.append_element(
        Some(container),
        "div",
        Some("display:block;min-block-size:40px;break-inside:avoid"),
    );
    // A rounded box taller than a column would be cut, and painting cannot
    // slice its corners, so the children are not sliced, and keep the layout
    // of the unfragmented flow.
    doc.append_element(
        Some(container),
        "div",
        Some("display:block;height:70px;border:2px solid;border-radius:2px"),
    );
    let half = doc.append_element(
        Some(container),
        "div",
        Some("display:block;width:50%;height:10px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 300.0;
    page.height = 200.0;
    layout_single_page(&mut doc, &cascade, page).expect("layout Ok");

    assert!(doc.layout_fragments().is_empty());
    assert_eq!(doc.nodes[half].layout.size.width, 100.0);
}

#[test]
fn ancestor_width_table_preserves_contents_percentage_and_min_content() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    crate::layout::test_support::with_ahem(&mut doc);
    let body = doc.append_element(Some(0), "body", Some("width:400px"));
    let contents = doc.append_element(Some(body), "div", Some("display:contents;width:80px"));
    let half = doc.append_element(Some(contents), "div", Some("width:50%"));
    let intrinsic = doc.append_element(Some(half), "div", Some("display:block;width:min-content"));
    let text = doc.append_text(intrinsic, "test");
    let cascade = cascade(&doc, &build_rule_tree(&doc)).expect("cascade");
    crate::layout::bridge::apply_computed_to_style(&mut doc, &cascade).expect("bridge");
    let widths = AncestorFacts::new(&doc, &cascade, 800.0);
    assert_eq!(widths.authored_containing_width(half), Some(400.0));
    assert_eq!(widths.authored_containing_width(text), Some(200.0));
    assert!(!widths.in_intrinsic_width(half));
    assert!(widths.in_intrinsic_width(intrinsic));
    assert!(widths.in_intrinsic_width(text));
    assert_eq!(widths.authored_containing_width(doc.nodes.len()), None);
}

#[test]
fn ancestor_width_table_treats_max_and_fit_content_as_intrinsic() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    crate::layout::test_support::with_ahem(&mut doc);
    let body = doc.append_element(Some(0), "body", Some("width:400px"));
    let max = doc.append_element(Some(body), "div", Some("display:block;width:max-content"));
    let max_text = doc.append_text(max, "test");
    let fit = doc.append_element(Some(body), "div", Some("display:block;width:fit-content"));
    let fit_text = doc.append_text(fit, "test");
    // The limit of the function does not make the width an authored one.
    let limited = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:fit-content(50px)"),
    );
    let limited_text = doc.append_text(limited, "test");
    let cascade = cascade(&doc, &build_rule_tree(&doc)).expect("cascade");
    crate::layout::bridge::apply_computed_to_style(&mut doc, &cascade).expect("bridge");
    let widths = AncestorFacts::new(&doc, &cascade, 800.0);
    for (node, text) in [(max, max_text), (fit, fit_text), (limited, limited_text)] {
        assert!(widths.in_intrinsic_width(node));
        assert!(widths.in_intrinsic_width(text));
        // A keyword width is not an authored containing width: the text
        // looks further up, to the body.
        assert_eq!(widths.authored_containing_width(text), Some(400.0));
    }
}

#[test]
fn ancestor_width_table_ignores_a_keyword_width_on_a_contents_element() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    crate::layout::test_support::with_ahem(&mut doc);
    let body = doc.append_element(Some(0), "body", Some("width:400px"));
    let mut texts = Vec::new();
    for keyword in ["min-content", "max-content", "fit-content"] {
        let contents = doc.append_element(
            Some(body),
            "div",
            Some(&format!("display:contents;width:{keyword}")),
        );
        texts.push((contents, doc.append_text(contents, "test")));
    }
    let cascade = cascade(&doc, &build_rule_tree(&doc)).expect("cascade");
    crate::layout::bridge::apply_computed_to_style(&mut doc, &cascade).expect("bridge");
    let widths = AncestorFacts::new(&doc, &cascade, 800.0);
    // The element has no box, so nothing below it is inside an intrinsic width.
    for (contents, text) in texts {
        assert!(!widths.in_intrinsic_width(contents));
        assert!(!widths.in_intrinsic_width(text));
    }
}

#[test]
fn ancestor_width_table_ignores_a_keyword_width_on_a_non_replaced_inline() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    crate::layout::test_support::with_ahem(&mut doc);
    let body = doc.append_element(Some(0), "body", Some("width:400px"));
    let mut boxes = Vec::new();
    // `width` does not apply to an inline box, but it does once the box is
    // blockified, atomic or floated.
    for (css, applies) in [
        ("display:inline", false),
        ("display:inline;float:left", true),
        ("display:inline-block", true),
        ("display:inline;position:absolute", true),
    ] {
        for keyword in ["min-content", "max-content", "fit-content"] {
            let outer =
                doc.append_element(Some(body), "span", Some(&format!("{css};width:{keyword}")));
            let block = doc.append_element(Some(outer), "div", Some("display:block"));
            let text = doc.append_text(block, "test");
            boxes.push((css, keyword, applies, outer, text));
        }
    }
    let cascade = cascade(&doc, &build_rule_tree(&doc)).expect("cascade");
    crate::layout::bridge::apply_computed_to_style(&mut doc, &cascade).expect("bridge");
    let widths = AncestorFacts::new(&doc, &cascade, 800.0);
    for (css, keyword, applies, outer, text) in boxes {
        assert_eq!(widths.in_intrinsic_width(outer), applies, "{css} {keyword}");
        assert_eq!(widths.in_intrinsic_width(text), applies, "{css} {keyword}");
    }
}

#[test]
fn ancestor_width_table_counts_a_keyword_width_on_a_flex_item() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    crate::layout::test_support::with_ahem(&mut doc);
    let body = doc.append_element(Some(0), "body", Some("display:flex;width:400px"));
    // A flex item is sized by its keyword like any other box, so its text
    // takes the width the keyword gives.
    let item = doc.append_element(Some(body), "div", Some("display:block;width:max-content"));
    let text = doc.append_text(item, "test");
    let cascade = cascade(&doc, &build_rule_tree(&doc)).expect("cascade");
    crate::layout::bridge::apply_computed_to_style(&mut doc, &cascade).expect("bridge");
    let widths = AncestorFacts::new(&doc, &cascade, 800.0);
    assert!(widths.in_intrinsic_width(item));
    assert!(widths.in_intrinsic_width(text));
}

#[test]
fn ancestor_width_table_ignores_a_keyword_width_the_layout_does_not_use() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    crate::layout::test_support::with_ahem(&mut doc);
    let body = doc.append_element(Some(0), "body", Some("display:flex;width:400px"));
    // A replaced element takes its natural size, as if the width were `auto`.
    let image = doc.append_element(Some(body), "img", Some("display:block;width:max-content"));
    let cascade = cascade(&doc, &build_rule_tree(&doc)).expect("cascade");
    crate::layout::bridge::apply_computed_to_style(&mut doc, &cascade).expect("bridge");
    let widths = AncestorFacts::new(&doc, &cascade, 800.0);
    assert!(!widths.in_intrinsic_width(image));
}

#[test]
fn ancestor_width_table_counts_a_keyword_width_on_a_multicol_container() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    crate::layout::test_support::with_ahem(&mut doc);
    let body = doc.append_element(Some(0), "body", Some("display:flex;width:400px"));
    let columns = doc.append_element(
        Some(body),
        "div",
        Some("display:block;column-count:2;width:max-content"),
    );
    let text = doc.append_text(columns, "test");
    let cascade = cascade(&doc, &build_rule_tree(&doc)).expect("cascade");
    crate::layout::bridge::apply_computed_to_style(&mut doc, &cascade).expect("bridge");
    let widths = AncestorFacts::new(&doc, &cascade, 800.0);
    assert!(widths.in_intrinsic_width(columns));
    assert!(widths.in_intrinsic_width(text));
}

#[test]
fn ancestor_width_table_uses_cascade_resolved_ch_and_percentage_inputs() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let body = doc.append_element(Some(0), "body", None::<&str>);
    let half = doc.append_element(Some(body), "div", Some("width:50%"));
    let text = doc.append_text(half, "percentage");
    let ch = doc.append_element(Some(body), "div", Some("width:4ch"));
    let ch_text = doc.append_text(ch, "ch");
    doc.set_font_collection(crate::layout::ifc::test_support::ahem_fonts());
    let cascade = cascade(&doc, &build_rule_tree(&doc)).expect("cascade");
    apply_computed_to_style(&mut doc, &cascade).expect("styles");
    assert_eq!(
        cascade.computed[ch].width,
        ComputedLengthPercentageOrAuto::Px(64.0)
    );
    let resolved_ch = style_dimension_length(
        crate::layout::test_support::bridged_style(&mut doc, &cascade, ch)
            .size
            .width,
    )
    .expect("cascade-resolved ch width should be a finite length");
    assert_eq!(resolved_ch, 64.0);
    for page_width in [800.0, 400.0, 800.0] {
        let rebreak = AncestorFacts::new(&doc, &cascade, page_width);
        assert_eq!(
            rebreak.authored_containing_width(text),
            Some(page_width / 2.0)
        );
        assert_eq!(
            rebreak.authored_containing_width(ch_text),
            Some(resolved_ch)
        );
    }
}

#[test]
fn parent_lookup_matches_arena_edges_through_detached_and_fragment_mutations() {
    fn assert_parents(doc: &Document) {
        let mut from_children = vec![None; doc.nodes.len()];
        for (parent, node) in doc.nodes.iter().enumerate() {
            for &child in &node.children {
                from_children[child] = Some(parent);
            }
        }
        for (child, expected) in from_children.into_iter().enumerate() {
            assert_eq!(doc.parent_of(child), expected);
        }
    }

    let mut doc = Document::new();
    let body = doc.append_element(Some(0), "body", None::<&str>);
    let branch = doc.append_element(Some(body), "div", None::<&str>);
    let child = doc.append_element(Some(branch), "div", None::<&str>);
    let text = doc.append_text(child, "detached text");
    let late_parent = doc.append_element(Some(body), "div", None::<&str>);
    doc.detach_from_parent(branch);
    doc.mark_in_document_flags();
    assert!(!doc.nodes[branch].is_in_document());
    assert_eq!(doc.parent_of(branch), None);
    assert_eq!(doc.parent_of(child), Some(branch));
    assert_eq!(doc.parent_of(text), Some(child));
    assert_parents(&doc);

    doc.attach_child(late_parent, branch);
    assert_parents(&doc);
    let template = doc.append_element(Some(body), "template", None::<&str>);
    let fragment = doc.allocate_template_fragment_root(template);
    let fragment_child = doc.append_element(Some(fragment), "div", None::<&str>);
    doc.append_text(fragment_child, "template text");
    doc.mark_in_document_flags();
    assert_eq!(doc.parent_of(fragment), None);
    assert_eq!(doc.parent_of(fragment_child), Some(fragment));
    assert!(!doc.nodes[fragment_child].is_in_document());
    assert_parents(&doc);

    doc.attach_child(body, fragment);
    assert_eq!(doc.parent_of(fragment), None);
    assert_eq!(doc.parent_of(fragment_child), Some(body));
    assert_parents(&doc);
    doc.reparent_children(branch, late_parent);
    assert_eq!(doc.parent_of(child), Some(late_parent));
    assert_parents(&doc);
    doc.retain_children(|id| id != child);
    assert_eq!(doc.parent_of(child), None);
    assert_eq!(doc.parent_of(text), Some(child));
    assert_parents(&doc);
}

#[test]
fn a_paragraph_continued_in_a_later_column_starts_at_its_top() {
    // Orphans keep the paragraph out of the first column, and widows leave
    // the second one a line short: the third column still starts with the
    // fifth line at its top.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let multicol = doc.append_element(
        Some(body),
        "div",
        Some(
            "display:block;width:300px;columns:3;column-gap:0;column-fill:auto;height:50px;\
             orphans:3;widows:3;font-size:10px;line-height:10px",
        ),
    );
    let first = doc.append_element(Some(multicol), "div", Some("display:block"));
    for _ in 0..3 {
        doc.append_text(first, "a");
        doc.append_element(Some(first), "br", None::<&str>);
    }
    let second = doc.append_element(Some(multicol), "div", Some("display:block"));
    for _ in 0..7 {
        doc.append_text(second, "b");
        doc.append_element(Some(second), "br", None::<&str>);
    }
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade Ok");
    let mut page = raikiri_traits::PageBox::new();
    page.width = 800.0;
    page.height = 600.0;
    layout_single_page(&mut doc, &cascade, page).expect("layout Ok");

    let ranges: Vec<_> = doc.nodes[second]
        .ifc_multicol_fragments()
        .expect("column ranges")
        .iter()
        .map(|range| (range.line_start, range.line_end, range.fragmentainer))
        .collect();
    assert_eq!(ranges, [(0, 4, 1), (4, 7, 2)]);
    let line_tops = |column| {
        let positioned =
            crate::PositionedLines::new(&doc, &cascade, second, Some(column)).expect("lines");
        positioned
            .all_lines()
            .iter()
            .enumerate()
            .filter_map(|(index, line)| {
                Some(positioned.line_offset(index)?.1 + line.block_offset())
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(line_tops(1), [0.0, 10.0, 20.0, 30.0]);
    assert_eq!(line_tops(2), [0.0, 10.0, 20.0]);
    // Each column holds a fragment of the paragraph at its top; the first
    // one runs to the column's end.
    let fragments: Vec<_> = doc
        .fragment_tree
        .fragments
        .iter()
        .filter(|fragment| fragment.node_id == second)
        .map(|fragment| {
            (
                fragment.fragmentainer,
                fragment.rect.y,
                fragment.rect.height,
            )
        })
        .collect();
    assert_eq!(fragments, [(1, 0.0, 50.0), (2, 0.0, 30.0)]);
}

#[test]
fn a_nested_column_container_keeps_its_own_fragment_height() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let multicol = doc.append_element(
        Some(body),
        "div",
        Some(
            "display:block;width:300px;columns:3;column-gap:0;column-fill:auto;height:80px;\
             font-size:10px;line-height:10px",
        ),
    );
    let first = doc.append_element(Some(multicol), "div", Some("display:block"));
    for _ in 0..2 {
        doc.append_text(first, "a");
        doc.append_element(Some(first), "br", None::<&str>);
    }
    let nested = doc.append_element(
        Some(multicol),
        "div",
        Some("display:block;columns:2;column-gap:0;height:20px"),
    );
    for _ in 0..4 {
        doc.append_text(nested, "b");
        doc.append_element(Some(nested), "br", None::<&str>);
    }
    layout_nested_flex_float_fixture(&mut doc);

    let heights: Vec<_> = doc
        .fragment_tree
        .fragments
        .iter()
        .filter(|fragment| fragment.node_id == nested)
        .map(|fragment| fragment.rect.height)
        .collect();
    assert_eq!(heights.first(), Some(&20.0));
}

#[test]
fn a_nested_multicol_whose_last_row_and_bottom_padding_overflow_the_outer_column_is_not_continued_in_rows()
 {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let outer = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:200px;height:100px;columns:2;column-gap:0;column-fill:auto"),
    );
    let inner = doc.append_element(
        Some(outer),
        "div",
        Some("display:block;columns:2;column-gap:0;padding-bottom:10px"),
    );
    doc.append_element(Some(inner), "div", Some("display:block;height:400px"));
    layout_nested_flex_float_fixture(&mut doc);
    // The last row fills the outer column, so its bottom padding has no room.
    assert!(doc.nodes[inner].multicol_rows.is_empty());

    doc.set_element_inline_style(inner, Some("display:block;columns:2;column-gap:0".into()));
    layout_nested_flex_float_fixture(&mut doc);
    assert_eq!(doc.nodes[inner].multicol_rows.len(), 2);
}

/// A multicol container whose children are each a containing block for the
/// absolutely positioned boxes listed with it.
fn absolute_fixture(
    container_css: &str,
    children: &[(&str, &[&str])],
) -> (Document, usize, Vec<(usize, Vec<usize>)>) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let parent = doc.append_element(Some(body), "div", Some(container_css));
    let ids = children
        .iter()
        .map(|(css, boxes)| {
            let child = doc.append_element(Some(parent), "div", Some(*css));
            let boxes = boxes
                .iter()
                .map(|css| doc.append_element(Some(child), "div", Some(*css)))
                .collect();
            (child, boxes)
        })
        .collect();
    layout_nested_flex_float_fixture(&mut doc);
    (doc, parent, ids)
}

fn fragment_clips(doc: &Document, node: usize) -> Vec<crate::fragment::FragmentRect> {
    doc.fragment_tree
        .fragments
        .iter()
        .filter(|fragment| fragment.node_id == node)
        .filter_map(|fragment| fragment.fragmentainer_clip)
        .collect()
}

const DEFINITE_COLUMNS: &str =
    "display:block;width:200px;height:100px;columns:2;column-gap:0;column-fill:auto";

#[test]
fn an_absolute_box_breaks_at_the_column_ends_of_its_containing_block() {
    let (doc, _, ids) = absolute_fixture(
        DEFINITE_COLUMNS,
        &[(
            "display:block;position:relative;height:50px",
            &["position:absolute;top:30px;width:10px;height:100px"],
        )],
    );
    let (block, boxes) = &ids[0];
    let block_record = doc
        .fragment_tree
        .fragments
        .iter()
        .position(|fragment| fragment.node_id == *block)
        .unwrap();
    // Each piece is the whole box, moved into its column and clipped to it.
    assert_eq!(
        fragment_rects(&doc, boxes[0]),
        vec![(0, 0.0, 30.0, 100.0), (1, 100.0, -70.0, 100.0)]
    );
    assert!(
        doc.fragment_tree
            .fragments
            .iter()
            .filter(|fragment| fragment.node_id == boxes[0])
            .all(|fragment| fragment.parent == Some(block_record))
    );
    assert_eq!(
        fragment_clips(&doc, boxes[0]),
        vec![
            crate::fragment::FragmentRect {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
            crate::fragment::FragmentRect {
                x: 100.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
        ]
    );
}

#[test]
fn an_absolute_box_past_the_last_column_continues_in_overflow_columns() {
    let (doc, _, ids) = absolute_fixture(
        DEFINITE_COLUMNS,
        &[(
            "display:block;position:relative;height:10px",
            &["position:absolute;top:0;width:10px;height:250px"],
        )],
    );
    let columns: Vec<usize> = fragment_rects(&doc, ids[0].1[0])
        .into_iter()
        .map(|(column, ..)| column)
        .collect();
    assert_eq!(columns, vec![0, 1, 2]);
}

#[test]
fn an_absolute_box_breaks_where_its_overflow_below_its_border_ends() {
    let (mut doc, _, ids) = absolute_fixture(
        DEFINITE_COLUMNS,
        &[(
            "display:block;position:relative;height:10px",
            &["position:absolute;top:0;width:10px;height:0;border-top:10px solid"],
        )],
    );
    let absolute = ids[0].1[0];
    // The child sits below the border, so the box reaches 105px down.
    doc.append_element(Some(absolute), "div", Some("height:95px"));
    layout_nested_flex_float_fixture(&mut doc);
    let columns: Vec<usize> = fragment_rects(&doc, absolute)
        .into_iter()
        .map(|(column, ..)| column)
        .collect();
    assert_eq!(columns, vec![0, 1]);
}

#[test]
fn an_absolute_box_starts_where_its_overflow_above_it_starts() {
    let (mut doc, _, ids) = absolute_fixture(
        DEFINITE_COLUMNS,
        &[(
            "display:block;position:relative;height:150px",
            &["position:absolute;top:110px;width:10px;height:10px"],
        )],
    );
    let absolute = ids[0].1[0];
    // The child is moved up into the first column.
    doc.append_element(
        Some(absolute),
        "div",
        Some("position:relative;top:-30px;height:10px"),
    );
    layout_nested_flex_float_fixture(&mut doc);
    assert_eq!(
        fragment_rects(&doc, absolute),
        vec![(0, 0.0, 110.0, 10.0), (1, 0.0, 10.0, 10.0)]
    );
}

#[test]
fn the_last_column_of_an_unlimited_container_holds_the_rest_of_an_absolute_box() {
    let (doc, _, ids) = absolute_fixture(
        "display:block;width:200px;columns:2;column-gap:0",
        &[(
            "display:block;position:relative;height:100px",
            &["position:absolute;top:0;width:10px;height:300px"],
        )],
    );
    // The columns balance at 50px; the second, last one grows to hold the
    // 250px left of the box.
    assert_eq!(
        fragment_rects(&doc, ids[0].1[0]),
        vec![(0, 0.0, 0.0, 300.0), (1, 0.0, -50.0, 300.0)]
    );
    assert_eq!(fragment_clips(&doc, ids[0].1[0])[1].height, 250.0);
}

#[test]
fn absolute_boxes_the_columns_cannot_hold_keep_the_other_strategies() {
    let unbroken = |block: &str, absolute: &str| {
        let (doc, _, ids) = absolute_fixture(DEFINITE_COLUMNS, &[(block, &[absolute])]);
        fragment_rects(&doc, ids[0].1[0]).is_empty()
    };
    let block = "display:block;position:relative;height:50px";
    // A box that starts above the columns.
    assert!(unbroken(
        block,
        "position:absolute;top:-10px;width:10px;height:20px"
    ));
    // Pieces that would not fit in the fragment budget.
    assert!(unbroken(
        block,
        "position:absolute;top:0;width:10px;height:10000000px",
    ));
}

#[test]
fn a_cut_containing_block_holds_absolute_pieces_in_the_columns_after_its_own() {
    // The 150px block ends 50px down the second column; the 250px box
    // continues in an overflow column.
    let (doc, _, ids) = absolute_fixture(
        DEFINITE_COLUMNS,
        &[(
            "display:block;position:relative;height:150px",
            &["position:absolute;top:0;width:10px;height:250px"],
        )],
    );
    let (block, absolute) = (ids[0].0, ids[0].1[0]);
    let fragments = |node: usize| {
        doc.fragment_tree
            .fragments
            .iter()
            .enumerate()
            .filter(move |(_, fragment)| fragment.node_id == node)
    };
    let pieces: Vec<_> = fragments(absolute)
        .map(|(_, fragment)| (fragment.fragmentainer, fragment.parent))
        .collect();
    let block_pieces: Vec<_> = fragments(block).map(|(id, _)| Some(id)).collect();
    assert_eq!(
        pieces,
        vec![
            (0, block_pieces[0]),
            (1, block_pieces[1]),
            (2, block_pieces[1])
        ]
    );
    // The block's last piece shows the box in the column after it.
    let clip = doc.fragment_tree.fragments[block_pieces[1].unwrap()]
        .fragmentainer_clip
        .unwrap();
    assert_eq!((clip.x, clip.width), (100.0, 200.0));
}

#[test]
fn only_a_positioned_block_breaks_the_absolute_boxes_it_contains() {
    let supported = |block: &str, absolute: &str, out_of_flow: bool| {
        let (mut doc, parent, _) = absolute_fixture(DEFINITE_COLUMNS, &[(block, &[absolute])]);
        with_bridged_session(&mut doc, |session| {
            lineless::supported(session, parent, out_of_flow)
        })
        .is_some()
    };
    let absolute = "position:absolute;top:0;width:10px;height:20px";
    let block = "display:block;position:relative;height:50px";
    assert!(supported(block, absolute, true));
    assert!(!supported(block, absolute, false));
    // A static block is not their containing block.
    assert!(!supported("display:block;height:50px", absolute, true));
    // A transformed block contains fixed boxes too.
    assert!(!supported(
        "display:block;position:relative;height:50px;transform:scale(2)",
        absolute,
        true,
    ));
    // A fixed box has the viewport as its containing block.
    assert!(!supported(
        block,
        "position:fixed;top:0;width:10px;height:20px",
        true
    ));
}

#[test]
fn a_containing_block_that_holds_blocks_breaks_with_them_and_its_absolute_boxes() {
    let (doc, _, ids) = absolute_fixture(
        DEFINITE_COLUMNS,
        &[
            ("display:block;height:90px", &[]),
            (
                "display:block;position:relative",
                &[
                    "display:block;height:20px",
                    "position:absolute;top:0;width:10px;height:20px",
                ],
            ),
        ],
    );
    let (wrapper, boxes) = &ids[1];
    // The wrapper's pieces span its block's pieces in each column.
    assert_eq!(
        fragment_rects(&doc, *wrapper),
        vec![(0, 0.0, 90.0, 10.0), (1, 100.0, 0.0, 10.0)]
    );
    let wrapper_records: Vec<usize> = doc
        .fragment_tree
        .fragments
        .iter()
        .enumerate()
        .filter(|(_, fragment)| fragment.node_id == *wrapper)
        .map(|(index, _)| index)
        .collect();
    for node in boxes {
        let parents: Vec<Option<usize>> = doc
            .fragment_tree
            .fragments
            .iter()
            .filter(|fragment| fragment.node_id == *node)
            .map(|fragment| fragment.parent)
            .collect();
        assert_eq!(
            parents,
            wrapper_records
                .iter()
                .copied()
                .map(Some)
                .collect::<Vec<_>>()
        );
    }
    assert_eq!(
        fragment_rects(&doc, boxes[0]),
        vec![(0, 0.0, 0.0, 10.0), (1, 0.0, 0.0, 10.0)]
    );
    assert_eq!(
        fragment_rects(&doc, boxes[1]),
        vec![(0, 0.0, 0.0, 20.0), (1, 0.0, -10.0, 20.0)]
    );
}

#[test]
fn only_a_plain_containing_block_holds_blocks_in_the_column_stream() {
    let supported = |block: &str, out_of_flow: bool| {
        let (mut doc, parent, _) = absolute_fixture(
            DEFINITE_COLUMNS,
            &[(
                block,
                &[
                    "display:block;height:20px",
                    "position:absolute;top:0;width:10px;height:20px",
                ],
            )],
        );
        with_bridged_session(&mut doc, |session| {
            lineless::supported(session, parent, out_of_flow)
        })
        .is_some()
    };
    let block = "display:block;position:relative";
    assert!(supported(block, true));
    assert!(!supported(block, false));
    // Blocks in a box that is not their absolute boxes' containing block
    // keep the other strategies.
    assert!(!supported("display:block", true));
    // A wrapper's borders, padding and own block size are not sliced.
    assert!(!supported(
        "display:block;position:relative;border-top:1px solid",
        true
    ));
    assert!(!supported(
        "display:block;position:relative;height:50px",
        true
    ));
    assert!(!supported(
        "display:block;position:relative;max-height:50px",
        true
    ));
    assert!(!supported(
        "display:block;position:relative;min-height:100%",
        true
    ));
    assert!(supported(
        "display:block;position:relative;min-height:0",
        true
    ));
    assert!(!supported(
        "display:block;position:relative;break-inside:avoid",
        true
    ));
    // A block with content of its own has lines or boxes to break between.
    let (mut doc, parent, ids) = absolute_fixture(
        DEFINITE_COLUMNS,
        &[(
            block,
            &[
                "display:block;height:20px",
                "position:absolute;top:0;width:10px;height:20px",
            ],
        )],
    );
    doc.append_element(Some(ids[0].1[0]), "div", Some("height:10px"));
    assert!(
        with_bridged_session(&mut doc, |session| lineless::supported(
            session, parent, true
        ))
        .is_none()
    );
}

#[test]
fn only_a_first_child_at_the_top_of_a_multicol_starts_the_outer_columns() {
    let starts = |outer: &str, before: Option<&str>, nested: &str| {
        let children: Vec<(&str, &[&str])> = before
            .into_iter()
            .chain([nested])
            .map(|css| (css, &[] as &[&str]))
            .collect();
        let (mut doc, _, ids) = absolute_fixture(outer, &children);
        with_bridged_session(&mut doc, |session| {
            lineless::starts_the_outer_columns(session, ids.last().unwrap().0)
        })
    };
    let nested = "display:block;columns:2;height:50px";
    assert!(starts(DEFINITE_COLUMNS, None, nested));
    // Earlier content, a top margin or a top padding moves it down.
    assert!(!starts(DEFINITE_COLUMNS, Some("height:40px"), nested));
    assert!(!starts(
        DEFINITE_COLUMNS,
        None,
        "columns:2;height:50px;margin-top:10px"
    ));
    assert!(!starts(
        &format!("{DEFINITE_COLUMNS};padding-top:10px"),
        None,
        nested
    ));
    // So does text before it, which collapsible white space is not.
    let text_before_in = |outer: &str, text: &str| {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", None::<&str>);
        let body = doc.append_element(Some(html), "body", Some("margin:0"));
        let parent = doc.append_element(Some(body), "div", Some(outer));
        doc.append_text(parent, text);
        let nested = doc.append_element(Some(parent), "div", Some(nested));
        layout_nested_flex_float_fixture(&mut doc);
        with_bridged_session(&mut doc, |session| {
            lineless::starts_the_outer_columns(session, nested)
        })
    };
    let text_before = |text: &str| text_before_in(DEFINITE_COLUMNS, text);
    assert!(!text_before("text"));
    assert!(text_before("  "));
    // A no-break space and preserved white space make a line.
    assert!(!text_before("\u{a0}"));
    assert!(!text_before_in(
        &format!("{DEFINITE_COLUMNS};white-space:pre"),
        " \n "
    ));
    // A parent that is not a multicol container may itself sit anywhere.
    assert!(!starts("display:block;height:100px", None, nested));
    // The root has no parent.
    let mut doc = Document::new();
    assert!(!with_bridged_session(&mut doc, |session| {
        lineless::starts_the_outer_columns(session, 0)
    }));
}

#[test]
fn only_in_flow_paragraphs_with_content_heights_count_as_paragraph_children() {
    let paragraphs = |child_style: &str| {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", None::<&str>);
        let body = doc.append_element(Some(html), "body", Some("margin:0"));
        let parent = doc.append_element(
            Some(body),
            "div",
            Some("display:block;width:100px;height:40px;column-count:2;column-gap:20px"),
        );
        let first = doc.append_element(Some(parent), "p", Some("display:block;margin:0"));
        doc.append_text(first, "A");
        let second = doc.append_element(Some(parent), "p", Some(child_style));
        doc.append_text(second, "B");
        layout_nested_flex_float_fixture(&mut doc);
        with_bridged_session(&mut doc, |session| {
            multicol_has_only_paragraph_children(session, parent)
        })
    };
    assert!(paragraphs("display:block;margin:0"));
    // A single paragraph keeps the existing path.
    assert!(!paragraphs("display:none"));
    assert!(paragraphs(
        "display:block;margin:0 4px;padding-left:2px;border-right:2px solid"
    ));
    // A relative offset is taken out of the column location, so the planner
    // can place the paragraph.
    assert!(paragraphs(
        "display:block;margin:0;position:relative;left:10px;right:-10px"
    ));
    for style in [
        "display:grid",
        "display:list-item",
        "display:block;position:absolute",
        "display:block;float:left",
        "display:block;height:20px",
        "display:block;min-height:20px",
        "display:block;max-height:20px",
        "display:block;padding-top:2px",
        "display:block;border-bottom:2px solid",
        "display:block;margin-top:20px",
        "display:block;margin-bottom:20px",
        "display:block;margin:0;writing-mode:vertical-rl",
        "display:block;margin:0;position:sticky;left:10px",
        "display:block;margin:0;position:relative;top:5px",
    ] {
        assert!(!paragraphs(style), "{style}");
    }
}

#[test]
fn an_absolute_box_in_an_empty_containing_block_starts_in_an_overflow_column() {
    let (doc, _, ids) = absolute_fixture(
        "display:block;width:100px;height:100px;columns:2;column-gap:0;column-fill:auto",
        &[(
            "display:block;position:relative",
            &["position:absolute;top:300px;width:50px;height:200px"],
        )],
    );
    assert_eq!(
        fragment_rects(&doc, ids[0].1[0]),
        vec![(3, 150.0, 0.0, 200.0), (4, 200.0, -100.0, 200.0)]
    );
}

#[test]
fn an_absolute_box_in_an_empty_containing_block_starts_in_the_first_column() {
    let (doc, _, ids) = absolute_fixture(
        DEFINITE_COLUMNS,
        &[(
            "display:block;position:relative",
            &["position:absolute;top:0;width:10px;height:250px"],
        )],
    );
    assert_eq!(
        fragment_rects(&doc, ids[0].1[0]),
        vec![
            (0, 0.0, 0.0, 250.0),
            (1, 100.0, -100.0, 250.0),
            (2, 200.0, -200.0, 250.0),
        ]
    );
}

#[test]
fn a_containing_block_with_no_block_size_is_sliced_for_the_absolute_boxes_it_holds() {
    let supported = |block: &str, absolute: Option<&str>, out_of_flow: bool| {
        let boxes: Vec<&str> = absolute.into_iter().collect();
        let (mut doc, parent, _) = absolute_fixture(DEFINITE_COLUMNS, &[(block, &boxes)]);
        with_bridged_session(&mut doc, |session| {
            lineless::supported(session, parent, out_of_flow)
        })
        .is_some()
    };
    let absolute = "position:absolute;top:0;width:10px;height:20px";
    let block = "display:block;position:relative";
    assert!(supported(block, Some(absolute), true));
    assert!(!supported(block, Some(absolute), false));
    // Without an absolute box there is nothing to break.
    assert!(!supported(block, None, true));
    // A static block is not their containing block.
    assert!(!supported("display:block", Some(absolute), true));
    // A fixed box has the viewport as its containing block.
    assert!(!supported(
        block,
        Some("position:fixed;top:0;width:10px;height:20px"),
        true,
    ));
}

#[test]
fn a_relative_column_child_is_placed_without_its_inset() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    // The painter adds the inset of a box that is placed in a column, so
    // the location layout gives it is the column's own.
    for inset in ["left:5px", "right:5px", "left:10%", "left:5px;top:3px"] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", None::<&str>);
        let body = doc.append_element(Some(html), "body", None::<&str>);
        let container = doc.append_element(
            Some(body),
            "div",
            Some("display:block;column-count:2;column-gap:20px;width:100px;height:40px;column-fill:auto"),
        );
        let child = doc.append_element(
            Some(container),
            "div",
            Some(format!("display:block;position:relative;{inset}").as_str()),
        );
        doc.append_text(child, "A B C");

        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade Ok");
        layout_single_page(&mut doc, &cascade, PageBox::A4).expect("layout Ok");

        let layout = doc.nodes[child].layout;
        assert_eq!(layout.location.x, 0.0, "{inset}");
        assert_eq!(layout.size.width, 40.0, "{inset}");
    }
}

#[test]
fn a_relative_column_child_that_holds_boxes_is_placed_without_its_inset() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    // A wrapper holding a paragraph, and a nested column container, are
    // moved by the painter together with everything inside them, so the
    // location layout gives them is the column's own as well.
    for inner_style in [
        "display:block",
        "display:block;column-count:2;column-gap:10px",
    ] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", None::<&str>);
        let body = doc.append_element(Some(html), "body", None::<&str>);
        let container = doc.append_element(
            Some(body),
            "div",
            Some("display:block;column-count:2;column-gap:20px;width:100px;height:40px;column-fill:auto"),
        );
        let child = doc.append_element(
            Some(container),
            "div",
            Some(format!("{inner_style};position:relative;left:5px").as_str()),
        );
        let paragraph = doc.append_element(Some(child), "p", Some("display:block"));
        doc.append_text(paragraph, "A B C");

        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade Ok");
        layout_single_page(&mut doc, &cascade, PageBox::A4).expect("layout Ok");

        let layout = doc.nodes[child].layout;
        assert_eq!(layout.location.x, 0.0, "{inner_style}");
        assert_eq!(layout.size.width, 40.0, "{inner_style}");
    }
}

#[test]
fn authored_auto_block_width_accounts_for_box_sizing_and_decorations() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    for css in [
        "padding:0 10px",
        "padding:0 10%",
        "border-left:5px solid;border-right:5px solid",
        "box-sizing:border-box;padding:0 10px;border-left:5px solid;border-right:5px solid",
    ] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", None::<&str>);
        let body = doc.append_element(Some(html), "body", Some("margin:0;width:100px"));
        let block = doc.append_element(
            Some(body),
            "div",
            Some(format!("display:block;width:auto;{css}").as_str()),
        );
        doc.append_element(Some(block), "div", Some("height:1px"));
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade Ok");
        let mut page = PageBox::new();
        page.width = 100.0;
        page.height = 100.0;
        layout_single_page(&mut doc, &cascade, page).expect("layout Ok");

        assert_eq!(doc.nodes[block].layout.size.width, 100.0, "{css}");
    }
}

#[test]
fn authored_auto_block_width_propagates_content_width_to_auto_descendants() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;width:100px"),
    );
    let outer = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:auto;padding:0 10px"),
    );
    let inner = doc.append_element(Some(outer), "div", Some("display:block;width:auto"));
    doc.append_element(Some(inner), "div", Some("height:1px"));

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    layout_single_page(&mut doc, &cascade, page).expect("layout Ok");

    assert_eq!(doc.nodes[outer].layout.size.width, 100.0);
    assert_eq!(doc.nodes[inner].layout.size.width, 80.0);
}

#[test]
fn authored_auto_block_width_min_width_is_preserved_for_auto_descendants() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;width:100px"),
    );
    let outer = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:auto;min-width:100px;padding:0 10px"),
    );
    let inner = doc.append_element(Some(outer), "div", Some("display:block;width:auto"));
    doc.append_element(Some(inner), "div", Some("height:1px"));

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    layout_single_page(&mut doc, &cascade, page).expect("layout Ok");

    assert_eq!(doc.nodes[outer].layout.size.width, 120.0);
    assert_eq!(doc.nodes[inner].layout.size.width, 100.0);
}

#[test]
fn authored_auto_block_width_max_width_is_propagated_to_auto_descendants() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;width:100px"),
    );
    let outer = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:auto;max-width:50px;padding:0 10px"),
    );
    let inner = doc.append_element(Some(outer), "div", Some("display:block;width:auto"));
    doc.append_element(Some(inner), "div", Some("height:1px"));

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    layout_single_page(&mut doc, &cascade, page).expect("layout Ok");

    assert_eq!(doc.nodes[outer].layout.size.width, 70.0);
    assert_eq!(doc.nodes[inner].layout.size.width, 50.0);
}

#[test]
fn authored_auto_block_width_constraints_follow_border_box_sizing() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    for (constraint, outer_width, inner_width) in [
        ("min-width:120px", 120.0, 100.0),
        ("max-width:80px", 80.0, 60.0),
    ] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", None::<&str>);
        let body = doc.append_element(
            Some(html),
            "body",
            Some("display:block;margin:0;width:100px"),
        );
        let outer = doc.append_element(
            Some(body),
            "div",
            Some(
                format!(
                    "display:block;width:auto;box-sizing:border-box;padding:0 10px;{constraint}"
                )
                .as_str(),
            ),
        );
        let inner = doc.append_element(Some(outer), "div", Some("display:block;width:auto"));
        doc.append_element(Some(inner), "div", Some("height:1px"));

        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade Ok");
        let mut page = PageBox::new();
        page.width = 100.0;
        page.height = 100.0;
        layout_single_page(&mut doc, &cascade, page).expect("layout Ok");

        assert_eq!(doc.nodes[outer].layout.size.width, outer_width);
        assert_eq!(doc.nodes[inner].layout.size.width, inner_width);
    }
}

#[test]
fn a_block_whose_height_follows_from_its_ratio_is_sliced_into_definite_height_columns() {
    let (doc, _, children) = lineless_fixture(
        "display:block;width:100px;height:100px;columns:4;column-gap:0;column-fill:auto",
        &["display:block;width:25px;aspect-ratio:1/16"],
    );
    assert_eq!(
        fragment_rects(&doc, children[0]),
        vec![
            (0, 0.0, 0.0, 100.0),
            (1, 25.0, 0.0, 100.0),
            (2, 50.0, 0.0, 100.0),
            (3, 75.0, 0.0, 100.0)
        ]
    );
}

#[test]
fn a_block_whose_height_follows_from_its_ratio_is_balanced_across_the_columns() {
    let (doc, parent, children) = lineless_fixture(
        "display:block;width:100px;columns:4;column-gap:0",
        &["display:block;width:25px;aspect-ratio:1/16"],
    );
    assert_eq!(doc.nodes[parent].layout.size.height, 100.0);
    assert_eq!(
        fragment_rects(&doc, children[0]),
        vec![
            (0, 0.0, 0.0, 100.0),
            (1, 25.0, 0.0, 100.0),
            (2, 50.0, 0.0, 100.0),
            (3, 75.0, 0.0, 100.0)
        ]
    );
}

#[test]
fn a_block_that_fills_the_column_takes_its_ratio_height_from_the_column_width() {
    let (doc, parent, children) = lineless_fixture(
        "display:block;width:200px;columns:2;column-gap:0",
        &["display:block;aspect-ratio:2/1"],
    );
    // 50px tall at the 100px width of a column, balanced into two columns.
    assert_eq!(doc.nodes[parent].layout.size.height, 25.0);
    assert_eq!(
        fragment_rects(&doc, children[0]),
        vec![(0, 0.0, 0.0, 25.0), (1, 100.0, 0.0, 25.0)]
    );
}

#[test]
fn authored_box_reads_what_the_style_view_reads() {
    let close = |a: f32, b: f32| (a - b).abs() <= 1e-3;
    let close_option = |a: Option<f32>, b: Option<f32>| match (a, b) {
        (Some(a), Some(b)) => close(a, b),
        (a, b) => a == b,
    };
    // `(tag, css)` of a box inside a 200px wide parent. The parent and the
    // body are plain blocks.
    let cases = [
        ("div", ""),
        ("div", "box-sizing:border-box;padding:10px;border:3px solid"),
        ("div", "padding:10%;border:2px solid"),
        (
            "div",
            "padding:calc(5px + 10%) calc(2px + 1%);border:1px solid",
        ),
        ("div", "padding-left:-3px"),
        ("div", "border-left:4px solid;border-right:6px solid"),
        (
            "div",
            "border-left:4px solid;border-right:6px solid;width:50px",
        ),
        (
            "div",
            "border-left:4px solid;border-right:6px solid;padding-top:3px",
        ),
        (
            "div",
            "border-left:4px solid;border-right:6px solid;padding-top:0%",
        ),
        ("div", "border-left:4px solid;border-top:1px solid"),
        ("div", "min-width:30px;max-width:80px"),
        ("div", "min-width:25%;max-width:calc(50% - 4px)"),
        ("div", "min-width:min-content;max-width:fit-content"),
        ("div", "min-width:max-content;max-width:max-content"),
        ("div", "min-width:auto;max-width:none"),
        ("div", "float:left"),
        ("div", "float:right"),
        ("div", "float:none"),
        ("div", "float:inline-start"),
        ("div", "float:inline-end"),
        ("div", "float:footnote"),
        ("div", "aspect-ratio:2"),
        ("div", "aspect-ratio:auto 3/2"),
        ("div", "aspect-ratio:auto"),
        ("div", "contain:size;aspect-ratio:2"),
        ("div", "contain:size;width:50px;aspect-ratio:auto 2"),
        ("div", "contain:inline-size;aspect-ratio:2"),
        ("img", "width:30px"),
        ("img", "width:30px;height:20px"),
        ("img", "width:auto;height:auto"),
        (
            "div",
            "writing-mode:vertical-rl;border-left:4px solid;border-right:6px solid",
        ),
        ("div", "writing-mode:vertical-lr;width:50px;aspect-ratio:2"),
        (
            "div",
            "width:min-content;border-left:4px solid;border-right:6px solid",
        ),
        (
            "div",
            "width:max-content;border-left:4px solid;border-right:6px solid",
        ),
        (
            "div",
            "width:fit-content;border-left:4px solid;border-right:6px solid",
        ),
        (
            "div",
            "display:inline;width:fit-content;border-left:4px solid",
        ),
        (
            "div",
            "display:inline-block;width:max-content;border-left:4px solid",
        ),
        (
            "div",
            "display:flex;width:max-content;border-left:4px solid",
        ),
        (
            "div",
            "display:table;width:min-content;border-left:4px solid",
        ),
        (
            "img",
            "width:min-content;border-left:4px solid;border-right:6px solid",
        ),
        (
            "img",
            "width:max-content;border-left:4px solid;border-right:6px solid",
        ),
        (
            "img",
            "writing-mode:vertical-rl;inline-size:max-content;aspect-ratio:2",
        ),
        (
            "img",
            "writing-mode:vertical-rl;inline-size:max-content;contain:size;aspect-ratio:2",
        ),
        (
            "div",
            "writing-mode:vertical-rl;inline-size:max-content;contain:size;aspect-ratio:2",
        ),
        ("div", "contain:size;width:min-content;aspect-ratio:2"),
        (
            "div",
            "contain:size;width:fit-content;height:fit-content;aspect-ratio:2",
        ),
        (
            "div",
            "display:inline-block;contain:size;width:max-content;aspect-ratio:2",
        ),
        (
            "div",
            "writing-mode:vertical-rl;contain:size;height:min-content;aspect-ratio:2",
        ),
        (
            "div",
            "writing-mode:vertical-rl;contain:size;width:100px;height:fit-content;aspect-ratio:2",
        ),
        (
            "div",
            "writing-mode:vertical-rl;height:max-content;border-top:4px solid",
        ),
    ];
    for (tag, css) in cases {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", None::<&str>);
        let body = doc.append_element(Some(html), "body", Some("margin:0"));
        let parent = doc.append_element(Some(body), "div", Some("display:block;width:200px"));
        let boxed = doc.append_element(
            Some(parent),
            tag,
            Some(format!("display:block;{css}").as_str()),
        );
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade Ok");
        let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
        apply_computed_to_style_in_session(&mut tree, &cascade).expect("styles");
        let view = tree.query_style(boxed, ReadRole::CoreContainer);
        let authored = AuthoredBox::new(&tree, &cascade, boxed);
        let ctx = format!("{tag} {{{css}}}");

        assert_eq!(
            authored.is_border_box(),
            view.box_sizing() == BoxSizing::BorderBox,
            "box-sizing of {ctx}"
        );
        assert_eq!(
            authored.is_floated(),
            view.float().is_floated(),
            "float of {ctx}"
        );
        assert_eq!(
            authored.is_replaced(),
            view.is_replaced(),
            "replaced of {ctx}"
        );
        assert_eq!(
            authored.has_preferred_aspect_ratio(),
            view.aspect_ratio().is_some(),
            "aspect ratio of {ctx}"
        );
        for basis in [0.0, 37.5, 200.0, f32::INFINITY] {
            let padding = view.padding();
            let border = view.border();
            let edges = multicol_resolve_inset(&tree, padding.left, basis)
                + multicol_resolve_inset(&tree, padding.right, basis)
                + multicol_resolve_inset(&tree, border.left, basis)
                + multicol_resolve_inset(&tree, border.right, basis);
            assert!(
                close(authored.horizontal_edges(basis), edges),
                "horizontal edges of {ctx} against {basis}"
            );
            assert!(
                close_option(
                    authored.min_width(basis),
                    multicol_definite_dimension(&tree, view.min_size().width.into(), Some(basis))
                ),
                "min-width of {ctx} against {basis}"
            );
            assert!(
                close_option(
                    authored.max_width(basis),
                    multicol_definite_dimension(&tree, view.max_size().width.into(), Some(basis))
                ),
                "max-width of {ctx} against {basis}"
            );
        }
    }
}

/// A chain of `depth` column containers with a definite height, each in the
/// one above, around some text, and the containers from the outermost in.
fn nested_columns_with_a_height(depth: usize) -> (Document, Vec<usize>) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let mut parent = body;
    let mut containers = Vec::new();
    for _ in 0..depth {
        parent = doc.append_element(
            Some(parent),
            "div",
            Some("display:block;column-count:2;column-gap:10px;height:5000px"),
        );
        containers.push(parent);
    }
    doc.append_text(parent, "text");
    (doc, containers)
}

/// Lays the document out on a page of the given size in Ahem.
fn lay_out_in_ahem(doc: &mut Document, width: f32, height: f32) {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    doc.set_font_collection(crate::layout::test_support::ifc_ahem_fonts());
    let rules = build_rule_tree(doc);
    let cascade = cascade(doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = width;
    page.height = height;
    layout_single_page(doc, &cascade, page).expect("layout Ok");
}

/// The width the lines of the paragraph were last broken at.
fn lines_broken_at(doc: &Document, paragraph: usize) -> f32 {
    doc.nodes[paragraph]
        .ifc
        .as_ref()
        .and_then(|root| root.lines.as_ref())
        .expect("lines of the paragraph")
        .width
}

/// Text that takes several lines of Ahem in a column of 395px.
const COLUMN_TEXT: &str = "beta zeta sigma xi xi sigma beta beta nu omicron mu mu sigma sigma alpha \
    lambda gamma theta delta mu mu delta nu delta mu pi lambda beta mu gamma lambda pi eta eta epsilon";

#[test]
fn nested_columns_with_a_height_are_laid_out_once_per_level() {
    // Each container with a height laid its children out across its whole
    // width and again in its columns, once for each layout of the container
    // above, so the layouts doubled with every level: a chain of twelve took
    // 1779 of them.
    let depth = 12;
    let (mut doc, containers) = nested_columns_with_a_height(depth);
    crate::layout::test_support::MULTICOL_LAYOUTS.with(|count| count.set(0));
    lay_out_in_ahem(&mut doc, 800.0, 600.0);
    let layouts = crate::layout::test_support::MULTICOL_LAYOUTS.with(std::cell::Cell::get);
    assert!(
        layouts <= 8 * depth,
        "{layouts} layouts of {depth} containers"
    );

    // Every container is as tall as it states, and as wide as a column of the
    // one above while that is wide enough to tell.
    let mut width = 800.0;
    for (level, &container) in containers.iter().enumerate() {
        let size = doc.nodes[container].layout.size;
        assert!((size.height - 5000.0).abs() < 0.01, "{size:?}");
        if level < 5 {
            assert!((size.width - width).abs() < 0.01, "{size:?} in {width}");
        }
        width = (width - 10.0) / 2.0;
    }
}

#[test]
fn text_in_columns_of_a_container_laid_out_again_takes_the_width_of_its_column() {
    // The columns of the outer container lay the container with a height out
    // again; the text in it is laid out at the width of its own columns, not
    // at the one the first layout across the whole width gave it.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0;font:10px/10px Ahem"));
    let outer = doc.append_element(Some(body), "div", Some("display:block;column-count:1"));
    let columns = doc.append_element(
        Some(outer),
        "div",
        Some("display:block;column-count:2;column-gap:10px;height:200px"),
    );
    let block = doc.append_element(Some(columns), "div", Some("display:block"));
    doc.append_text(block, COLUMN_TEXT);
    lay_out_in_ahem(&mut doc, 800.0, 600.0);
    assert_eq!(doc.nodes[block].layout.size.width, 395.0);
    assert_eq!(lines_broken_at(&doc, block), 395.0);
}

#[test]
fn text_in_columns_laid_out_after_a_skipped_container_is_laid_out_again() {
    // The container with a height inside the first one is left for the layout
    // in columns, so the paragraphs beside it were laid out across the whole
    // width in between: what the caches kept of them for the width of a
    // column no longer holds, and the lines would keep that width.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0;font:10px/10px Ahem"));
    let outer = doc.append_element(
        Some(body),
        "div",
        Some("display:block;column-count:1;height:500px"),
    );
    let wrapper = doc.append_element(Some(outer), "div", Some("display:block"));
    let middle = doc.append_element(
        Some(wrapper),
        "div",
        Some("display:block;column-count:2;column-gap:10px"),
    );
    doc.append_element(
        Some(middle),
        "div",
        Some("display:block;column-count:2;column-gap:10px;height:60px"),
    );
    let paragraph = doc.append_element(Some(middle), "div", Some("display:block"));
    doc.append_text(paragraph, COLUMN_TEXT);
    lay_out_in_ahem(&mut doc, 800.0, 600.0);
    assert_eq!(doc.nodes[paragraph].layout.size.width, 395.0);
    assert_eq!(lines_broken_at(&doc, paragraph), 395.0);
}

/// A part of the content of a column container: lines of text in Ahem 20px
/// high, a paragraph of that many lines, a column spanner of 20px with more
/// style, or a box with a style.
enum Part<'a> {
    Lines(usize),
    Paragraph(usize),
    Spanner(&'a str),
    Box(&'a str),
}

/// A column container of 100px with two columns whose own content is `parts`,
/// in a document of its own. The container, and the boxes among the parts in
/// order.
fn column_container(container_css: &str, parts: &[Part<'_>]) -> (Document, usize, Vec<usize>) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0;font:20px/20px Ahem"));
    let container = doc.append_element(
        Some(body),
        "div",
        Some(
            format!(
                "display:block;width:100px;column-count:2;column-gap:20px;orphans:1;widows:1;{container_css}"
            )
            .as_str(),
        ),
    );
    let mut boxes = Vec::new();
    for part in parts {
        match part {
            Part::Lines(count) => {
                for line in 0..*count {
                    doc.append_text(container, "x");
                    if line + 1 < *count {
                        doc.append_element(Some(container), "br", None::<&str>);
                    }
                }
            }
            Part::Paragraph(count) => {
                let paragraph = doc.append_element(Some(container), "div", Some("display:block"));
                for line in 0..*count {
                    doc.append_text(paragraph, "x");
                    if line + 1 < *count {
                        doc.append_element(Some(paragraph), "br", None::<&str>);
                    }
                }
            }
            Part::Spanner(css) => boxes.push(doc.append_element(
                Some(container),
                "div",
                Some(format!("display:block;column-span:all;height:20px;{css}").as_str()),
            )),
            Part::Box(css) => {
                boxes.push(doc.append_element(Some(container), "div", Some(*css)));
            }
        }
    }
    (doc, container, boxes)
}

/// The line ranges of the paragraph of `container` in its columns, as
/// `(first line, end, column, x, y)`.
fn column_ranges(doc: &Document, container: usize) -> Vec<(usize, usize, usize, f32, f32)> {
    doc.nodes[container]
        .ifc_multicol_fragments()
        .expect("lines in columns")
        .iter()
        .map(|range| {
            (
                range.line_start,
                range.line_end,
                range.fragmentainer,
                range.x,
                range.y,
            )
        })
        .collect()
}

#[test]
fn the_lines_around_a_spanner_fill_the_columns_group_by_group() {
    // Two lines in each column of a group: 40px of columns, the spanner, and
    // 40px of columns again. The lines used to stay in one column.
    let (mut doc, container, boxes) =
        column_container("", &[Part::Lines(4), Part::Spanner(""), Part::Lines(4)]);
    lay_out_in_ahem(&mut doc, 180.0, 160.0);
    assert_eq!(doc.nodes[container].layout.size.height, 100.0);
    assert_eq!(
        column_ranges(&doc, container),
        [
            (0, 2, 0, 0.0, 0.0),
            (2, 4, 1, 60.0, 0.0),
            (4, 6, 0, 0.0, 60.0),
            (6, 8, 1, 60.0, 60.0),
        ]
    );
    let layout = doc.nodes[boxes[0]].layout;
    assert_eq!(layout.location.x, 0.0);
    assert_eq!(layout.location.y, 40.0);
    assert_eq!(layout.size.width, 100.0);
    assert_eq!(layout.size.height, 20.0);
}

#[test]
fn a_group_with_one_line_fills_one_column() {
    let (mut doc, container, boxes) =
        column_container("", &[Part::Lines(1), Part::Spanner(""), Part::Lines(3)]);
    lay_out_in_ahem(&mut doc, 180.0, 160.0);
    assert_eq!(
        column_ranges(&doc, container),
        [
            (0, 1, 0, 0.0, 0.0),
            (1, 3, 0, 0.0, 40.0),
            (3, 4, 1, 60.0, 40.0),
        ]
    );
    assert_eq!(doc.nodes[boxes[0]].layout.location.y, 20.0);
    assert_eq!(doc.nodes[container].layout.size.height, 80.0);
}

#[test]
fn a_spanner_at_either_end_of_the_lines_has_no_group_on_that_side() {
    let (mut doc, container, boxes) = column_container("", &[Part::Spanner(""), Part::Lines(4)]);
    lay_out_in_ahem(&mut doc, 180.0, 160.0);
    assert_eq!(
        column_ranges(&doc, container),
        [(0, 2, 0, 0.0, 20.0), (2, 4, 1, 60.0, 20.0)]
    );
    assert_eq!(doc.nodes[boxes[0]].layout.location.y, 0.0);
    assert_eq!(doc.nodes[container].layout.size.height, 60.0);

    let (mut doc, container, boxes) = column_container("", &[Part::Lines(4), Part::Spanner("")]);
    lay_out_in_ahem(&mut doc, 180.0, 160.0);
    assert_eq!(
        column_ranges(&doc, container),
        [(0, 2, 0, 0.0, 0.0), (2, 4, 1, 60.0, 0.0)]
    );
    assert_eq!(doc.nodes[boxes[0]].layout.location.y, 40.0);
    assert_eq!(doc.nodes[container].layout.size.height, 60.0);
}

#[test]
fn the_margins_of_spanners_separate_them_from_the_groups_around_them() {
    // The margins between two spanners collapse; the ones next to a group stay.
    let (mut doc, container, boxes) = column_container(
        "",
        &[
            Part::Lines(4),
            Part::Spanner("margin-bottom:10px"),
            Part::Spanner("margin-top:20px"),
            Part::Lines(4),
        ],
    );
    lay_out_in_ahem(&mut doc, 180.0, 160.0);
    assert_eq!(doc.nodes[boxes[0]].layout.location.y, 40.0);
    assert_eq!(doc.nodes[boxes[1]].layout.location.y, 80.0);
    assert_eq!(column_ranges(&doc, container)[2], (4, 6, 0, 0.0, 100.0));
    assert_eq!(doc.nodes[container].layout.size.height, 140.0);
}

#[test]
fn only_the_group_after_the_last_spanner_fills_its_first_column_under_column_fill_auto() {
    // The group above a spanner is balanced; the last one is filled as
    // it would be without a spanner.
    let (mut doc, container, _) = column_container(
        "column-fill:auto",
        &[Part::Lines(4), Part::Spanner(""), Part::Lines(4)],
    );
    lay_out_in_ahem(&mut doc, 180.0, 160.0);
    assert_eq!(
        column_ranges(&doc, container),
        [
            (0, 2, 0, 0.0, 0.0),
            (2, 4, 1, 60.0, 0.0),
            (4, 8, 0, 0.0, 60.0),
        ]
    );
}

#[test]
fn the_groups_around_spanners_are_kept_for_the_rules_between_their_columns() {
    let (mut doc, container, _) =
        column_container("", &[Part::Lines(4), Part::Spanner(""), Part::Lines(4)]);
    lay_out_in_ahem(&mut doc, 180.0, 160.0);
    let groups: Vec<_> = doc.nodes[container]
        .multicol_groups
        .iter()
        .map(|group| {
            (
                group.context.origin_y,
                group.height,
                group.occupied.iter().copied().collect::<Vec<_>>(),
            )
        })
        .collect();
    assert_eq!(groups, [(0.0, 40.0, vec![0, 1]), (60.0, 40.0, vec![0, 1])]);
}

#[test]
fn a_paragraph_with_other_boxes_than_spanners_is_not_split_in_groups() {
    // A float, an absolutely positioned box and a plain block each keep the
    // paragraph in one column.
    for css in [
        "display:block;float:left;width:20px;height:20px",
        "display:block;position:absolute;width:20px;height:20px",
        "display:block;height:20px",
    ] {
        let (mut doc, container, _) = column_container(
            "",
            &[
                Part::Lines(4),
                Part::Box(css),
                Part::Spanner(""),
                Part::Lines(4),
            ],
        );
        lay_out_in_ahem(&mut doc, 180.0, 160.0);
        assert!(
            doc.nodes[container].multicol_groups.is_empty(),
            "groups with {css}"
        );
    }
}

#[test]
fn a_paragraph_in_the_columns_of_another_container_is_not_split_in_groups() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0;font:20px/20px Ahem"));
    let outer = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:300px;column-count:2;column-gap:20px"),
    );
    let inner = doc.append_element(
        Some(outer),
        "div",
        Some("display:block;column-count:2;column-gap:20px"),
    );
    doc.append_text(inner, "x");
    doc.append_element(Some(inner), "br", None::<&str>);
    doc.append_text(inner, "x");
    doc.append_element(
        Some(inner),
        "div",
        Some("display:block;column-span:all;height:20px"),
    );
    doc.append_text(inner, "x");
    lay_out_in_ahem(&mut doc, 400.0, 160.0);
    assert!(doc.nodes[inner].multicol_groups.is_empty());
}

#[test]
fn a_container_with_a_height_is_not_split_in_groups() {
    let (mut doc, container, _) = column_container(
        "height:200px",
        &[Part::Lines(4), Part::Spanner(""), Part::Lines(4)],
    );
    lay_out_in_ahem(&mut doc, 180.0, 400.0);
    assert!(doc.nodes[container].multicol_groups.is_empty());
}

/// A flex container that aligns its items by their baselines: a box of one
/// line, and a column container whose own content `fill` adds. The offset the
/// box ends up below the top of the column container is how far its baseline
/// lies below the first baseline of the columns.
fn baseline_offset_of_columns(fill: impl FnOnce(&mut Document, usize)) -> f32 {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0;font:20px/20px Ahem"));
    let flex = doc.append_element(
        Some(body),
        "div",
        Some("display:flex;align-items:baseline;width:300px"),
    );
    let item = doc.append_element(Some(flex), "div", Some("display:block"));
    doc.append_text(item, "x");
    let columns = doc.append_element(
        Some(flex),
        "div",
        Some("display:block;width:100px;column-count:2;column-gap:20px;orphans:1;widows:1"),
    );
    fill(&mut doc, columns);
    lay_out_in_ahem(&mut doc, 400.0, 400.0);
    doc.nodes[item].layout.location.y - doc.nodes[columns].layout.location.y
}

/// A spanner of the column container `columns` with the line "s" in it.
fn text_spanner(doc: &mut Document, columns: usize, css: &str) {
    let spanner = doc.append_element(
        Some(columns),
        "div",
        Some(format!("display:block;column-span:all;{css}").as_str()),
    );
    doc.append_text(spanner, "s");
}

fn bare_line(doc: &mut Document, columns: usize) {
    doc.append_text(columns, "x");
    doc.append_element(Some(columns), "br", None::<&str>);
}

#[test]
fn the_first_baseline_of_columns_is_that_of_a_line_before_the_first_spanner() {
    let offset = baseline_offset_of_columns(|doc, columns| {
        bare_line(doc, columns);
        bare_line(doc, columns);
        text_spanner(doc, columns, "");
        bare_line(doc, columns);
    });
    assert_eq!(offset, 0.0);
}

#[test]
fn the_first_baseline_of_columns_is_that_of_a_spanner_before_the_first_line() {
    // The spanner is 30px below the top of the columns, and the lines are 20px
    // below it.
    let offset = baseline_offset_of_columns(|doc, columns| {
        text_spanner(doc, columns, "margin-top:30px;margin-bottom:20px");
        bare_line(doc, columns);
        bare_line(doc, columns);
    });
    assert_eq!(offset, 30.0);
}

#[test]
fn a_spanner_without_a_baseline_leaves_the_first_baseline_to_the_lines_below_it() {
    let offset = baseline_offset_of_columns(|doc, columns| {
        doc.append_element(
            Some(columns),
            "div",
            Some("display:block;column-span:all;height:30px"),
        );
        bare_line(doc, columns);
        bare_line(doc, columns);
    });
    assert_eq!(offset, 30.0);
}

#[test]
fn an_inline_block_of_columns_sits_on_the_baseline_of_its_last_line() {
    // Two lines in each group, one in each column, around a spanner of 20px:
    // the last line stands 40px below the top, so its baseline lies 56px
    // below it, and the box beside the columns drops by as much less than
    // that as its own baseline is high.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0;font:20px/20px Ahem"));
    let columns = doc.append_element(
        Some(body),
        "div",
        Some("display:inline-block;width:100px;column-count:2;column-gap:20px;orphans:1;widows:1"),
    );
    bare_line(&mut doc, columns);
    doc.append_text(columns, "x");
    text_spanner(&mut doc, columns, "");
    bare_line(&mut doc, columns);
    doc.append_text(columns, "x");
    let beside = doc.append_element(Some(body), "div", Some("display:inline-block"));
    doc.append_text(beside, "x");
    lay_out_in_ahem(&mut doc, 400.0, 400.0);
    assert_eq!(doc.nodes[columns].layout.location.y, 0.0);
    assert_eq!(doc.nodes[beside].layout.location.y, 40.0);
}

#[test]
fn a_spanner_contains_its_floats() {
    // The spanner is a formatting context of its own, so it is as tall as the
    // float in it and the lines below it start under the float.
    let (mut doc, container, boxes) = column_container(
        "",
        &[Part::Lines(1), Part::Spanner("height:auto"), Part::Lines(2)],
    );
    doc.append_element(
        Some(boxes[0]),
        "div",
        Some("float:left;width:50px;height:40px"),
    );
    lay_out_in_ahem(&mut doc, 180.0, 160.0);
    assert_eq!(doc.nodes[boxes[0]].layout.size.height, 40.0);
    assert_eq!(doc.nodes[container].layout.size.height, 80.0);
    assert_eq!(
        column_ranges(&doc, container)
            .iter()
            .map(|range| range.4)
            .collect::<Vec<_>>(),
        [0.0, 60.0, 60.0]
    );
}

#[test]
fn the_margins_of_the_children_of_a_spanner_stay_inside_it() {
    // The 10px above the child does not collapse through the top of the
    // spanner, which stays 20px below the top of the columns.
    let (mut doc, container, boxes) = column_container(
        "",
        &[Part::Lines(1), Part::Spanner("height:auto"), Part::Lines(2)],
    );
    doc.append_element(Some(boxes[0]), "div", Some("margin-top:10px;height:20px"));
    lay_out_in_ahem(&mut doc, 180.0, 160.0);
    let spanner = doc.nodes[boxes[0]].layout;
    assert_eq!((spanner.location.y, spanner.size.height), (20.0, 30.0));
    assert_eq!(doc.nodes[container].layout.size.height, 70.0);
}

#[test]
fn a_spanner_in_a_relatively_positioned_inline_stays_across_the_columns() {
    // The lines move with the inline element, but the spanner is hoisted out
    // of it and spans the columns where it is, as in Chromium.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0;font:20px/20px Ahem"));
    let columns = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:100px;column-count:2;column-gap:20px;orphans:1;widows:1"),
    );
    let inline = doc.append_element(
        Some(columns),
        "span",
        Some("position:relative;left:10px;top:5px"),
    );
    bare_line(&mut doc, inline);
    doc.append_text(inline, "x");
    let spanner = doc.append_element(
        Some(inline),
        "div",
        Some("display:block;column-span:all;height:20px"),
    );
    bare_line(&mut doc, inline);
    doc.append_text(inline, "x");
    lay_out_in_ahem(&mut doc, 180.0, 160.0);
    let layout = doc.nodes[spanner].layout;
    assert_eq!(
        (
            layout.location.x,
            layout.location.y,
            layout.size.width,
            layout.size.height
        ),
        (0.0, 20.0, 100.0, 20.0)
    );
}

#[test]
fn a_narrow_spanner_in_columns_sits_where_the_direction_puts_it() {
    // The columns are 100px wide. An over-constrained spanner in a right to
    // left container changes its left margin (CSS 2.2 10.3.3), so it ends
    // 10px from the right edge when it has a margin of 10px there. A spanner
    // that overflows counts its auto margins as zero, so it is over-constrained
    // too and ends at the right edge. In a left to right container it starts
    // at the left one. The same holds with paragraphs around the spanner and
    // with lines of text around it.
    for (direction, css, left) in [
        ("rtl", "width:40px", 60.0),
        ("rtl", "width:40px;margin-right:10px", 50.0),
        ("rtl", "width:40px;margin-left:10px", 60.0),
        ("rtl", "width:40px;margin:0 auto", 30.0),
        ("rtl", "width:40px;margin-left:auto", 60.0),
        ("rtl", "width:40px;margin-right:auto", 0.0),
        ("rtl", "width:140px", -40.0),
        ("rtl", "width:140px;margin:0 auto", -40.0),
        ("ltr", "width:40px", 0.0),
        ("ltr", "width:40px;margin-left:10px", 10.0),
        ("ltr", "width:40px;margin:0 auto", 30.0),
        ("ltr", "width:140px;margin:0 auto", 0.0),
    ] {
        let container_css = format!("direction:{direction}");
        let (mut doc, _, boxes) = column_container(
            &container_css,
            &[Part::Lines(2), Part::Spanner(css), Part::Lines(2)],
        );
        lay_out_in_ahem(&mut doc, 180.0, 160.0);
        assert_eq!(
            doc.nodes[boxes[0]].layout.location.x, left,
            "lines {direction} {css}"
        );
        let (mut doc, _, boxes) = column_container(
            &container_css,
            &[Part::Paragraph(2), Part::Spanner(css), Part::Paragraph(2)],
        );
        lay_out_in_ahem(&mut doc, 180.0, 160.0);
        assert_eq!(
            doc.nodes[boxes[0]].layout.location.x, left,
            "paragraphs {direction} {css}"
        );
    }
}

#[test]
fn a_block_level_spanner_of_any_display_spans_the_columns_of_the_lines_around_it() {
    // Two lines in each column of a group: 40px of columns, the spanner of
    // 20px, and 40px of columns again, whatever the inner display of the box
    // is (CSS Multi-column Layout 1, 6.1).
    for display in ["flow-root", "flex", "grid", "table"] {
        let (mut doc, container, boxes) = column_container(
            "",
            &[
                Part::Lines(4),
                Part::Spanner(format!("display:{display}").as_str()),
                Part::Lines(4),
            ],
        );
        lay_out_in_ahem(&mut doc, 180.0, 160.0);
        assert_eq!(doc.nodes[container].layout.size.height, 100.0, "{display}");
        assert_eq!(
            column_ranges(&doc, container),
            [
                (0, 2, 0, 0.0, 0.0),
                (2, 4, 1, 60.0, 0.0),
                (4, 6, 0, 0.0, 60.0),
                (6, 8, 1, 60.0, 60.0),
            ],
            "{display}"
        );
        let layout = doc.nodes[boxes[0]].layout;
        assert_eq!(
            (layout.location.x, layout.location.y, layout.size.height),
            (0.0, 40.0, 20.0),
            "{display}"
        );
    }
}

#[test]
fn only_a_block_level_box_that_is_in_the_flow_is_a_spanner() {
    for (css, spans) in [
        ("display:block", true),
        ("display:list-item", true),
        ("display:flow-root", true),
        ("display:flex", true),
        ("display:grid", true),
        ("display:table", true),
        ("display:inline-block", false),
        ("display:inline-flex", false),
        ("display:inline-grid", false),
        ("display:inline-table", false),
        ("display:flow-root;float:left", false),
        ("display:flex;position:absolute", false),
    ] {
        let (mut doc, _, boxes) =
            column_container("", &[Part::Lines(2), Part::Spanner(css), Part::Lines(2)]);
        lay_out_in_ahem(&mut doc, 180.0, 160.0);
        assert_eq!(
            with_bridged_session(&mut doc, |session| spanning::is_spanner(session, boxes[0])),
            spans,
            "{css}"
        );
    }
}
