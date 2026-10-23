mod calc_owner_tests;
mod foundation_tests;
mod physical_memo_tests;
mod preparation_tests;
pub(super) mod scoped_override_tests;
pub(crate) use scoped_override_tests::with_setup_inspector;

use crate::layout::{bridge::apply_computed_to_style_in_session, context::LayoutSession, protocol};
use crate::{Document, LayoutStyle};
use raikiri_layout_style::{LengthProperty, OwnedPropertyOverride};
use raikiri_style::{ComputedLengthPercentageOrAuto as L, build_rule_tree, cascade};
use taffy::CoreStyle;
use taffy::Size as StyleSize; // taffy-boundary-lint:ignore: Size<Dimension> is style input

pub(crate) fn fixture() -> (Document, raikiri_style::CascadeResult, usize, usize) {
    fixture_with_styles("display:flex", "width:31px;height:7px")
}

pub(crate) fn fixture_with_styles(
    container: &str,
    child: &str,
) -> (Document, raikiri_style::CascadeResult, usize, usize) {
    let mut doc = Document::new();
    let root = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(root), "body", Some(container));
    let child = doc.append_element(Some(body), "div", Some(child));
    let text = doc.append_text(body, "text");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).unwrap();
    (doc, cascade, child, text)
}

#[test]
fn synthetic_recipe_matches_the_default_taffy_style() {
    let (mut doc, cascade, _, text) = fixture();
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    let view = session.style_view(text);
    let expected = LayoutStyle::default();
    assert_eq!(view.size(), expected.size());
    assert_eq!(view.margin(), expected.margin());
    assert_eq!(view.padding(), expected.padding());
    assert_eq!(view.is_block(), expected.is_block());
    assert_eq!(view.direction(), expected.direction());
    assert_eq!(view.box_sizing(), expected.box_sizing());
}

fn width(value: f32) -> Vec<OwnedPropertyOverride> {
    vec![OwnedPropertyOverride::length(LengthProperty::Width, L::Px(value), false).unwrap()]
}

#[test]
fn nested_override_restore_on_error_and_unwind() {
    let (mut doc, cascade, child, _) = fixture();
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    let result: Result<(), ()> = session.with_style_overrides(child, width(11.0), |tree| {
        assert_eq!(
            tree.style_view(child).size().width,
            taffy::Dimension::length(11.0)
        );
        let error: Result<(), ()> = tree.with_style_overrides(child, width(17.0), |tree| {
            assert_eq!(
                tree.style_view(child).size().width,
                taffy::Dimension::length(17.0)
            );
            Err(())
        });
        assert!(error.is_err());
        assert_eq!(
            tree.style_view(child).size().width,
            taffy::Dimension::length(11.0)
        );
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            tree.with_style_overrides(child, width(23.0), |tree| {
                assert_eq!(
                    tree.style_view(child).size().width,
                    taffy::Dimension::length(23.0)
                );
                panic!("scope unwind");
            });
        }));
        assert!(panic.is_err());
        assert_eq!(
            tree.style_view(child).size().width,
            taffy::Dimension::length(11.0)
        );
        Ok(())
    });
    assert!(result.is_ok());
    assert_eq!(
        session.style_view(child).size().width,
        taffy::Dimension::length(31.0)
    );
    assert_eq!(session.style_override_depth(child), 0);
}

#[test]
fn prepared_image_ratio_comes_from_intrinsic_facts() {
    let (mut doc, mut cascade, child, _) = fixture();
    let element = doc.nodes[child].data.as_element_mut().unwrap();
    element.tag_name = "img".into();
    element.image_intrinsic_size =
        Some(raikiri_traits::IntrinsicBox::new(40.0, 20.0).with_aspect_ratio(2.0));
    cascade.computed[child].width = L::Auto;
    cascade.computed[child].height = L::Auto;
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    assert_eq!(session.style_view(child).aspect_ratio(), Some(2.0));
}

#[test]
fn known_image_fallback_dimensions_are_prepared_as_sparse_overrides() {
    let (mut doc, mut cascade, child, _) = fixture();
    doc.nodes[child].data.as_element_mut().unwrap().tag_name = "img".into();
    doc.set_element_attribute(child, "src", "green.png")
        .unwrap();
    cascade.computed[child].width = L::Auto;
    cascade.computed[child].height = L::Auto;
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    assert_eq!(
        session.style_view(child).size().width,
        taffy::Dimension::length(100.0)
    );
}

#[test]
fn prepared_containment_clears_the_natural_ratio_override() {
    let (mut doc, mut cascade, child, _) = fixture();
    let element = doc.nodes[child].data.as_element_mut().unwrap();
    element.tag_name = "img".into();
    element.image_intrinsic_size =
        Some(raikiri_traits::IntrinsicBox::new(40.0, 20.0).with_aspect_ratio(2.0));
    cascade.computed[child].width = L::Auto;
    cascade.computed[child].height = L::Auto;
    cascade.computed[child].contain = raikiri_style::property::Contain::SIZE;
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    assert_eq!(session.style_view(child).aspect_ratio(), None);
}

#[test]
fn prepared_grid_alignment_preserves_the_semantic_bridge_overrides() {
    use taffy::AlignItems;
    for (container, child_css, align_self, justify_self, aspect_ratio) in [
        // A definite size keeps `stretch` from reaching the item.
        (
            "display:grid;align-items:stretch",
            "width:13px;height:max-content",
            Some(AlignItems::START),
            None,
            None,
        ),
        (
            "display:grid;align-items:stretch;justify-items:stretch",
            "aspect-ratio:2/1",
            None,
            None,
            Some(2.0),
        ),
        // `last baseline` aligns an absolutely positioned item to its end edge.
        (
            "display:grid;align-items:last baseline",
            "position:absolute;align-self:auto",
            Some(AlignItems::SAFE_SELF_END),
            None,
            None,
        ),
    ] {
        let (mut doc, cascade, child, _) = fixture_with_styles(container, child_css);
        let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
        apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
        let view = session.style_view(child);
        assert_eq!(
            taffy::GridItemStyle::align_self(&view),
            align_self,
            "{container}: {child_css}"
        );
        assert_eq!(
            taffy::GridItemStyle::justify_self(&view),
            justify_self,
            "{container}: {child_css}"
        );
        assert_eq!(
            view.aspect_ratio(),
            aspect_ratio,
            "{container}: {child_css}"
        );
    }
}

fn assert_neutral_getters(view: raikiri_layout_style::LayoutStyleView<'_>, expected: &LayoutStyle) {
    use taffy::{
        FlexboxContainerStyle as FC, FlexboxItemStyle as FI, GridContainerStyle as GC,
        GridItemStyle as GI,
    };
    macro_rules! same {
        ($trait:ident: $($getter:ident),+ $(,)?) => { $(assert_eq!($trait::$getter(&view), $trait::$getter(expected), "{}", stringify!($getter));)+ };
    }
    same!(CoreStyle: box_generation_mode, is_block, is_compressible_replaced, is_replaced, box_sizing, direction, overflow, scrollbar_width, position, inset, is_containing_block, size, min_size, max_size, aspect_ratio, margin, padding, border, contain);
    same!(FC: flex_direction, flex_wrap, gap, align_content, align_items, justify_content);
    same!(FI: flex_basis, flex_grow, flex_shrink, align_self);
    same!(GC: grid_auto_flow, gap, align_content, justify_content, align_items, justify_items, grid_template_area_row_count, grid_template_area_column_count);
    same!(GI: grid_row, grid_column, align_self, justify_self);
    assert_eq!(
        GC::grid_template_columns(&view).is_none(),
        GC::grid_template_columns(expected).is_none()
    );
    assert_eq!(
        GC::grid_template_rows(&view).is_none(),
        GC::grid_template_rows(expected).is_none()
    );
    assert_eq!(
        GC::grid_auto_columns(&view).count(),
        GC::grid_auto_columns(expected).count()
    );
    assert_eq!(
        GC::grid_auto_rows(&view).count(),
        GC::grid_auto_rows(expected).count()
    );
}

// These recipe tests explicitly refresh the published all-key view set.
fn prepare_for_inspection(tree: &mut LayoutSession<'_>) {
    let keys = tree.document.nodes.layout_keys().collect();
    tree.style_views
        .prepare_keys(tree.document, keys, &tree.scoped_overrides);
}

#[test]
fn contents_recipe_keeps_only_explicit_inherited_direction() {
    let (mut doc, cascade, child, _) = fixture_with_styles(
        "display:grid;direction:rtl",
        "display:contents;direction:rtl;width:31px;padding:7px;position:absolute",
    );
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
    prepare_for_inspection(&mut tree);
    assert_neutral_getters(
        tree.style_view(child),
        &LayoutStyle {
            direction: taffy::Direction::Rtl,
            ..LayoutStyle::default()
        },
    );
    block_view_tests::assert_all_new_roles(&tree, child);
}

#[test]
fn initial_containing_block_recipe_uses_page_dimensions() {
    let (mut doc, cascade, _, _) = fixture();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
    let icb = super::super::initial_containing_block::prepare(&mut tree, &cascade, 123.0, 456.0);
    prepare_for_inspection(&mut tree);
    assert_neutral_getters(
        tree.style_view(icb),
        &LayoutStyle {
            display: taffy::Display::Block,
            size: StyleSize {
                width: taffy::Dimension::length(123.0),
                height: taffy::Dimension::length(456.0),
            },
            ..LayoutStyle::default()
        },
    );
    block_view_tests::assert_all_new_roles(&tree, icb);
}

#[test]
fn anonymous_table_and_cell_recipes_inherit_only_selected_facts() {
    use crate::node::Node;
    use raikiri_style::property::DisplayValue;
    let (mut doc, cascade, child, _) =
        fixture_with_styles("display:block;direction:rtl", "width:31px;padding:9px");
    super::super::apply_computed_to_style(&mut doc, &cascade).unwrap();
    let parent = doc.nodes[child].parent.unwrap();
    let make = |display| {
        let mut node = Node::new_element("div".into(), None);
        node.parent = Some(parent);
        node.set_display(display);
        node
    };
    doc.set_anonymous_tables(vec![make(DisplayValue::Table)]);
    doc.set_anonymous_cells(vec![make(DisplayValue::TableCell)]);
    let table = doc.nodes.anonymous_table_key(0);
    let cell = doc.nodes.anonymous_cell_key(0);
    use crate::layout::box_style_data::recipes;
    // The fixture supplies the same explicit lowering owner for both boxes.
    for (idx, is_cell) in [(table, false), (cell, true)] {
        let facts = recipes::TableFacts::from_owner(&doc, &cascade, idx, parent);
        let recipe = if is_cell {
            recipes::anonymous_cell(facts)
        } else {
            recipes::anonymous_table(facts)
        };
        doc.box_style_data
            .publish_recipe(doc.nodes.box_key(idx).unwrap(), recipe);
    }
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    prepare_for_inspection(&mut tree);
    for idx in [table, cell] {
        assert_neutral_getters(
            tree.style_view(idx),
            &LayoutStyle {
                display: taffy::Display::Block,
                direction: taffy::Direction::Rtl,
                ..LayoutStyle::default()
            },
        );
        block_view_tests::assert_all_new_roles(&tree, idx);
    }
}

#[test]
fn resolved_edge_scopes_do_not_resolve_percentages_twice_and_restore_on_unwind() {
    let (mut doc, cascade, child, _) = fixture_with_styles(
        "display:flex",
        "display:flex;padding-left:calc(10% + 5px);padding-bottom:7px",
    );
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
    let original = tree.style_view(child).padding();
    let result: Result<(), ()> = tree.with_resolved_edges(child, 200.0, true, |tree| {
        let expected = taffy::LengthPercentage::length(25.0);
        assert_eq!(tree.style_view(child).padding().left, expected);
        assert_eq!(
            tree.style_view(child).padding().bottom,
            taffy::LengthPercentage::length(7.0)
        );
        tree.with_resolved_edges(child, 300.0, false, |tree| {
            assert_eq!(tree.style_view(child).padding().left, expected);
        });
        Err(())
    });
    assert!(result.is_err());
    assert_eq!(tree.style_view(child).padding(), original);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tree.with_resolved_edges(child, 100.0, false, |_| panic!("edge scope unwind"));
    }));
    assert!(panic.is_err());
    assert_eq!(tree.style_view(child).padding(), original);
    // The restored padding is the authored expression again: 10% + 5px.
    let pointer = tree
        .style_view(child)
        .padding()
        .left
        .into_raw()
        .calc_value();
    // SAFETY: The prepared style retains its owners.
    #[allow(unsafe_code)]
    unsafe {
        assert_eq!(raikiri_layout_style::resolve_calc(pointer, 100.0), 15.0);
    }
}

#[test]
fn viewport_overflow_is_recorded_when_the_body_write_happens() {
    let (mut doc, cascade, child, _) =
        fixture_with_styles("display:flex;overflow:hidden", "width:31px");
    let body = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
    assert_eq!(tree.style_view(body).overflow().x, taffy::Overflow::Hidden);
    tree.propagate_viewport_overflow(&cascade, body);
    assert_eq!(tree.style_view(body).overflow().x, taffy::Overflow::Visible);
    assert_eq!(
        cascade.computed[body].overflow.x,
        raikiri_style::property::OverflowValue::Hidden
    );
}

#[test]
fn fixed_and_vertical_direction_are_physical() {
    for (css, position, direction) in [
        (
            "position:fixed",
            taffy::Position::Absolute,
            taffy::Direction::Ltr,
        ),
        (
            "writing-mode:sideways-lr;direction:ltr",
            taffy::Position::Static,
            taffy::Direction::Rtl,
        ),
        (
            "writing-mode:vertical-rl;text-orientation:upright;direction:rtl",
            taffy::Position::Static,
            taffy::Direction::Ltr,
        ),
    ] {
        let (mut doc, cascade, child, _) = fixture_with_styles("display:flex", css);
        let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
        apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
        let view = session.style_view(child);
        assert_eq!(view.position(), position, "{css}");
        assert_eq!(view.direction(), direction, "{css}");
        assert_eq!(
            view.size(),
            LayoutStyle::default().size(),
            "physical sizes stay unprojected: {css}"
        );
    }
}

#[test]
fn source_metadata_covers_hidden_logical_and_keyword_policies() {
    for (parent, child_css) in [
        ("display:block", "display:none"),
        ("display:block", "display:contents"),
        (
            "display:block",
            "writing-mode:vertical-rl;inline-size:31px;block-size:19px",
        ),
        (
            "display:block",
            "display:flex;width:max-content;min-block-size:min-content",
        ),
        ("display:flex", "min-block-size:min-content"),
        ("display:flex", "width:max-content;min-width:1px"),
        (
            "display:flex",
            "writing-mode:vertical-rl;inline-size:max-content;block-size:max-content",
        ),
        (
            "display:block",
            "writing-mode:vertical-rl;inline-size:max-content;block-size:max-content",
        ),
    ] {
        let (mut doc, cascade, child, _) = fixture_with_styles(parent, child_css);
        let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
        apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
        prepare_for_inspection(&mut tree);
        assert_eq!(
            tree.style_view(child).box_generation_mode() == taffy::BoxGenerationMode::None,
            child_css == "display:none",
            "{child_css}"
        );
        block_view_tests::assert_all_new_roles(&tree, child);
        let crate::layout::box_style_data::LayoutBase::Synthetic(neutral) = &tree
            .box_style_data
            .get(crate::node::arena::BoxKey::Source(0))
            .unwrap()
            .base
        else {
            panic!("document root has a neutral recipe");
        };
        assert_eq!(neutral.width, L::Auto);
        assert_eq!(neutral.direction, raikiri_style::property::Direction::Ltr);
    }
}

#[test]
fn invalid_scope_order_and_republication_are_rejected_without_losing_state() {
    let (mut doc, cascade, child, _) = fixture();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
    let original = tree.style_view(child).size();
    let outer = tree.record_style_overrides(child, Vec::new());
    let prepare =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tree.prepare_style_views()));
    assert!(prepare.is_err());
    let bridge =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tree.begin_style_bridge()));
    assert!(bridge.is_err());
    let inner = tree.record_style_overrides(child, Vec::new());
    let order = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tree.restore_recorded_style_overrides(outer)
    }));
    assert!(order.is_err());
    tree.restore_recorded_style_overrides(inner);
    tree.restore_recorded_style_overrides(outer);
    let outer = tree.push_style_overrides(child, width(17.0));
    let inner = tree.push_style_overrides(child, width(23.0));
    let order = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tree.restore_style_overrides(outer)
    }));
    assert!(order.is_err());
    tree.restore_style_overrides(inner);
    tree.restore_style_overrides(outer);
    let unscoped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tree.record_scoped_length(child, LengthProperty::Width, L::Px(99.0), false)
    }));
    assert!(unscoped.is_err());
    assert_eq!(tree.style_override_depth(child), 0);
    assert_eq!(tree.style_view(child).size(), original);
}

#[test]
fn pass_override_publication_invalidates_parent_before_child_compute() {
    let (mut doc, cascade, child, _) = fixture();
    let parent = doc.nodes[child].parent.unwrap();
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    let key = session.nodes.box_key(parent).unwrap();
    let frame = session
        .layout_cache_context
        .enter_view(key, Default::default());
    let before = session.layout_cache_context.epoch();
    session.record_pass_override(child, width(55.0).pop().unwrap());
    assert!(session.layout_cache_context.epoch() > before);
    assert!(!session.layout_cache_context.can_store(key));
    assert_eq!(
        session.style_view(child).size().width,
        taffy::Dimension::length(55.0)
    );
    session.layout_cache_context.leave(frame);
}

#[test]
fn prepared_claims_include_transform_containing_blocks() {
    let (mut doc, cascade, child, _) = fixture_with_styles(
        "display:flex",
        "transform:translateX(0);width:31px;height:7px",
    );
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    let claims = session.style_view(child).is_containing_block();
    assert!(claims.absolute);
    assert!(claims.fixed);
}

#[test]
fn repeated_pass_overrides_replace_the_property() {
    let (mut doc, cascade, child, _) = fixture();
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    session.record_pass_override(child, width(44.0).pop().unwrap());
    session.record_pass_override(child, width(55.0).pop().unwrap());
    let key = session.nodes.box_key(child).unwrap();
    assert_eq!(session.box_style_data.get(key).unwrap().pass.len(), 1);
    assert_eq!(
        session.style_view(child).size().width,
        taffy::Dimension::length(55.0)
    );
}

#[test]
fn metadata_republication_prevents_ancestor_store() {
    let (mut doc, cascade, child, _) = fixture();
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    let key = session.nodes.box_key(child).unwrap();
    let frame = session
        .layout_cache_context
        .enter_view(key, Default::default());
    let before = session.layout_cache_context.epoch();
    session.nodes[child].claims_fixed_oof = true;
    session.prepare_style_views();
    assert!(session.layout_cache_context.epoch() > before);
    assert!(!session.layout_cache_context.can_store(key));
    assert!(session.style_view(child).is_containing_block().fixed);
    session.layout_cache_context.leave(frame);
}

#[test]
fn prepared_flags_keep_the_final_formatting_context() {
    use raikiri_layout_style::StyleFlags as F;
    for (css, flag) in [
        ("display:flex", F::FLEX),
        ("display:grid", F::GRID),
        ("display:table", F::TABLE),
        ("display:block;column-count:2", F::MULTICOL),
    ] {
        let (mut doc, cascade, child, _) = fixture_with_styles("display:flex", css);
        let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
        apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
        let key = session.nodes.box_key(child).unwrap();
        assert!(
            session
                .box_style_data
                .get(key)
                .unwrap()
                .metadata
                .flags
                .contains(flag),
            "{css}"
        );
    }
}

#[test]
fn fallback_image_replaced_policy_uses_normalized_keyword_sizes() {
    let (mut doc, cascade, child, _) =
        fixture_with_styles("display:flex", "width:max-content;height:20px");
    doc.nodes[child].data.as_element_mut().unwrap().tag_name = "img".into();
    doc.set_element_attribute(child, "src", "green.png")
        .unwrap();
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    assert_eq!(
        session.style_view(child).size().width,
        taffy::Dimension::length(100.0)
    );
    assert!(session.style_view(child).is_replaced());
}

#[test]
fn neutral_text_paragraph_publishes_its_final_ifc_classification() {
    let (mut doc, cascade, _, text) = fixture();
    crate::layout::test_support::with_ahem(&mut doc);
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    assert_eq!(
        session.nodes[text].formatting_context,
        crate::node::FormattingContext::Inline
    );
    let key = session.nodes.box_key(text).unwrap();
    assert!(
        session
            .box_style_data
            .get(key)
            .unwrap()
            .metadata
            .flags
            .contains(raikiri_layout_style::StyleFlags::IFC)
    );
}

#[test]
fn non_element_has_no_css_sizing_keyword_policy() {
    let (doc, mut cascade, _, text) = fixture();
    cascade.computed[text].width = L::MaxContent;
    cascade.computed[text].min_height = L::FitContent;
    assert_eq!(
        super::metadata::sizing_flags(&doc, &cascade, text),
        raikiri_layout_style::StyleFlags::EMPTY,
    );
}

#[test]
fn flex_grid_queries_preserve_legacy_claims_and_prepared_facts() {
    use taffy::{LayoutFlexboxContainer, LayoutGridContainer, LayoutPartialTree};
    for display in ["flex", "grid"] {
        for policy in ["transform:translateX(0)", "contain:layout size"] {
            for position in ["static", "relative"] {
                let css = format!("display:{display};{policy};position:{position}");
                let (mut doc, cascade, child, _) = fixture_with_styles("display:flex", &css);
                let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
                apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
                let id = protocol::NodeId::from(child);
                assert!(tree.style_view(child).is_containing_block().fixed);
                assert!(
                    tree.get_core_container_style(id)
                        .is_containing_block()
                        .fixed
                );
                let expected = taffy::ContainingBlockClaims {
                    absolute: position == "relative",
                    fixed: false,
                };
                assert_eq!(
                    tree.style_view(child).for_flex_grid().is_containing_block(),
                    expected
                );
                for actual in [
                    tree.get_flexbox_container_style(id).is_containing_block(),
                    tree.get_flexbox_child_style(id).is_containing_block(),
                    tree.get_grid_container_style(id).is_containing_block(),
                    tree.get_grid_child_style(id).is_containing_block(),
                ] {
                    assert_eq!(actual, expected);
                }
            }
        }
    }
}

pub(crate) fn prepare_all_layout_views(tree: &mut LayoutSession<'_>) {
    tree.publish_layout_cache_mutations();
    let keys = tree.nodes.layout_keys().collect();
    tree.style_views
        .prepare_keys(tree.document, keys, &tree.scoped_overrides);
}

mod block_view_tests;
