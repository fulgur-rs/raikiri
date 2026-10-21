mod calc_owner_tests;
mod foundation_tests;
mod preparation_tests;

use crate::layout::{bridge::apply_computed_to_style_in_session, context::LayoutSession};
use crate::{Document, LayoutStyle};
use raikiri_layout_style::{LengthProperty, OwnedPropertyOverride};
use raikiri_style::{ComputedLengthPercentageOrAuto as L, build_rule_tree, cascade};
use taffy::CoreStyle;

pub(crate) fn fixture() -> (Document, raikiri_style::CascadeResult, usize, usize) {
    fixture_with_styles("display:flex", "width:31px;height:7px")
}

pub(crate) fn fixture_with_styles(
    container: &str,
    child: &str,
) -> (Document, raikiri_style::CascadeResult, usize, usize) {
    let mut doc = Document::new();
    let root = doc.append_element(
        Some(0),
        "html",
        LayoutStyle::default(),
        Some("display:block"),
    );
    let body = doc.append_element(Some(root), "body", LayoutStyle::default(), Some(container));
    let child = doc.append_element(Some(body), "div", LayoutStyle::default(), Some(child));
    let text = doc.append_text(body, "text");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).unwrap();
    (doc, cascade, child, text)
}

#[test]
fn prepared_source_is_independent_of_retained_style() {
    let (mut doc, cascade, child, _) = fixture();
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    assert_eq!(
        session.style_view(child).size().width,
        taffy::Dimension::length(31.0)
    );
    session.layout_style_for_key_mut(child).unwrap().size.width = taffy::Dimension::length(99.0);
    assert_eq!(
        session.style_view(child).size().width,
        taffy::Dimension::length(31.0)
    );
}

#[test]
fn synthetic_recipe_matches_legacy_neutral_box() {
    let (mut doc, cascade, _, text) = fixture();
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    let view = session.style_view(text);
    let old = session.layout_style_for_key(text).unwrap();
    assert_eq!(view.size(), old.size());
    assert_eq!(view.margin(), old.margin());
    assert_eq!(view.padding(), old.padding());
    assert_eq!(view.is_block(), old.is_block());
    assert_eq!(view.direction(), old.direction());
    assert_eq!(view.box_sizing(), old.box_sizing());
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
    assert_eq!(
        session.layout_style_for_key(child).unwrap().aspect_ratio,
        Some(2.0)
    );
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
        session.style_view(child).size(),
        session.layout_style_for_key(child).unwrap().size
    );
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
    assert_eq!(
        session.layout_style_for_key(child).unwrap().aspect_ratio,
        None
    );
    assert_eq!(session.style_view(child).aspect_ratio(), None);
}

#[test]
fn prepared_grid_alignment_preserves_the_semantic_bridge_overrides() {
    for (container, child_css) in [
        (
            "display:grid;align-items:stretch",
            "width:13px;height:max-content",
        ),
        (
            "display:grid;align-items:stretch;justify-items:stretch",
            "aspect-ratio:2/1",
        ),
        (
            "display:grid;align-items:last baseline",
            "position:absolute;align-self:auto",
        ),
    ] {
        let (mut doc, cascade, child, _) = fixture_with_styles(container, child_css);
        let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
        apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
        let old = session.layout_style_for_key(child).unwrap();
        let view = session.style_view(child);
        assert_eq!(
            taffy::GridItemStyle::align_self(&view),
            old.align_self,
            "{container}: {child_css}"
        );
        assert_eq!(
            taffy::GridItemStyle::justify_self(&view),
            old.justify_self,
            "{container}: {child_css}"
        );
        assert_eq!(
            view.aspect_ratio(),
            old.aspect_ratio,
            "{container}: {child_css}"
        );
    }
}

fn assert_neutral_getters(view: raikiri_layout_style::LayoutStyleView<'_>, old: &LayoutStyle) {
    use taffy::{
        FlexboxContainerStyle as FC, FlexboxItemStyle as FI, GridContainerStyle as GC,
        GridItemStyle as GI,
    };
    macro_rules! same {
        ($trait:ident: $($getter:ident),+ $(,)?) => { $(assert_eq!($trait::$getter(&view), $trait::$getter(old), "{}", stringify!($getter));)+ };
    }
    same!(CoreStyle: box_generation_mode, is_block, is_compressible_replaced, is_replaced, box_sizing, direction, overflow, scrollbar_width, position, inset, is_containing_block, size, min_size, max_size, aspect_ratio, margin, padding, border, contain);
    same!(FC: flex_direction, flex_wrap, gap, align_content, align_items, justify_content);
    same!(FI: flex_basis, flex_grow, flex_shrink, align_self);
    same!(GC: grid_auto_flow, gap, align_content, justify_content, align_items, justify_items, grid_template_area_row_count, grid_template_area_column_count);
    same!(GI: grid_row, grid_column, align_self, justify_self);
    assert_eq!(
        GC::grid_template_columns(&view).is_none(),
        GC::grid_template_columns(old).is_none()
    );
    assert_eq!(
        GC::grid_template_rows(&view).is_none(),
        GC::grid_template_rows(old).is_none()
    );
    assert_eq!(
        GC::grid_auto_columns(&view).count(),
        GC::grid_auto_columns(old).count()
    );
    assert_eq!(
        GC::grid_auto_rows(&view).count(),
        GC::grid_auto_rows(old).count()
    );
}

// These recipe tests inspect boxes outside the production Flex/Grid query set.
fn prepare_for_inspection(tree: &mut LayoutSession<'_>) {
    let keys = tree.document.nodes.layout_keys().collect();
    tree.style_views
        .as_mut()
        .unwrap()
        .prepare_keys(tree.document, keys);
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
        tree.layout_style_for_key(child).unwrap(),
    );
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
        tree.layout_style_for_key(icb).unwrap(),
    );
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
        let mut style = LayoutStyle {
            direction: taffy::Direction::Rtl,
            ..LayoutStyle::default()
        };
        let mut node = Node::new_element("div".into(), style.clone(), None);
        node.parent = Some(parent);
        node.set_display(display);
        node.refresh_box_style(&mut style);
        (node, style)
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
            tree.layout_style_for_key(idx).unwrap(),
        );
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
    let original = tree.layout_style_for_key(child).unwrap().padding;
    let result: Result<(), ()> = tree.with_resolved_edges(child, 200.0, true, |tree| {
        let expected = taffy::LengthPercentage::length(25.0);
        assert_eq!(tree.style_view(child).padding().left, expected);
        assert_eq!(
            tree.layout_style_for_key(child).unwrap().padding.left,
            expected
        );
        assert_eq!(
            tree.style_view(child).padding().bottom,
            taffy::LengthPercentage::length(7.0)
        );
        tree.with_resolved_edges(child, 300.0, false, |tree| {
            assert_eq!(tree.style_view(child).padding().left, expected);
            assert_eq!(
                tree.layout_style_for_key(child).unwrap().padding.left,
                expected
            );
        });
        Err(())
    });
    assert!(result.is_err());
    assert_eq!(tree.layout_style_for_key(child).unwrap().padding, original);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tree.with_resolved_edges(child, 100.0, false, |_| panic!("edge scope unwind"));
    }));
    assert!(panic.is_err());
    assert_eq!(tree.layout_style_for_key(child).unwrap().padding, original);
    // The prepared and legacy handles differ; compare the restored expression by resolution.
    let view_pointer = tree
        .style_view(child)
        .padding()
        .left
        .into_raw()
        .calc_value();
    let old_pointer = original.left.into_raw().calc_value();
    // SAFETY: The prepared style and the document retain their respective owners.
    #[allow(unsafe_code)]
    unsafe {
        assert_eq!(
            raikiri_layout_style::resolve_calc(view_pointer, 100.0),
            crate::taffy_impl::resolve_calc(old_pointer, 100.0)
        );
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
    assert_eq!(
        tree.style_view(body).overflow(),
        tree.layout_style_for_key(body).unwrap().overflow
    );
    assert_eq!(tree.style_view(body).overflow().x, taffy::Overflow::Visible);
    assert_eq!(
        cascade.computed[body].overflow.x,
        raikiri_style::property::OverflowValue::Hidden
    );
}

#[test]
fn fixed_and_vertical_direction_match_physical_legacy_getters() {
    for css in [
        "position:fixed",
        "writing-mode:sideways-lr;direction:ltr",
        "writing-mode:vertical-rl;text-orientation:upright;direction:rtl",
    ] {
        let (mut doc, cascade, child, _) = fixture_with_styles("display:flex", css);
        let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
        apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
        let view = session.style_view(child);
        let old = session.layout_style_for_key(child).unwrap();
        assert_eq!(view.position(), old.position(), "{css}");
        assert_eq!(view.direction(), old.direction(), "{css}");
        assert_eq!(
            view.size(),
            old.size(),
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
        assert_neutral_getters(
            tree.style_view(child),
            tree.layout_style_for_key(child).unwrap(),
        );
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
    let outer = tree.save_size_scope(child);
    let prepare =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tree.prepare_style_views()));
    assert!(prepare.is_err());
    let bridge =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tree.begin_style_bridge()));
    assert!(bridge.is_err());
    let inner = tree.save_size_scope(child);
    let order = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tree.restore_size_scope(outer)
    }));
    assert!(order.is_err());
    tree.restore_size_scope(inner);
    tree.restore_size_scope(outer);
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
fn repeated_pass_overrides_replace_the_property_and_project_legacy_values() {
    let (mut doc, cascade, child, _) = fixture();
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    session.record_pass_override(child, width(44.0).pop().unwrap());
    session.record_pass_override(child, width(55.0).pop().unwrap());
    let key = session.nodes.box_key(child).unwrap();
    assert_eq!(session.box_style_data.get(key).unwrap().pass.len(), 1);
    assert_eq!(
        session.layout_style_for_key(child).unwrap().size.width,
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
    assert!(
        session
            .layout_style_for_key(child)
            .unwrap()
            .item_is_replaced
    );
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
