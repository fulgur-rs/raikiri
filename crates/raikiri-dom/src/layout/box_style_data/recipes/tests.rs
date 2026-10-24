use super::*;
use crate::LayoutStyle;
use raikiri_layout_style::values::{
    BoxGenerationMode, Dimension, Direction as LayoutDirection, Display, StyleSize,
};
use raikiri_layout_style::{
    LayoutStyleView, PreparedStyle, StyleFlags, StyleMetadata, TaffyStyle, ViewContext,
};
use raikiri_style::{
    ComputedBorderSpacing, ComputedLengthPercentageOrAuto as L,
    property::{BorderCollapseValue, CaptionSideValue, WritingMode},
};
use taffy::{
    CoreStyle, FlexboxContainerStyle as FC, FlexboxItemStyle as FI, GridContainerStyle as GC,
    GridItemStyle as GI,
};

fn facts(direction: LayoutDirection, block: bool) -> BoxFacts {
    BoxFacts {
        direction,
        metadata: StyleMetadata {
            flags: if block {
                StyleFlags::BLOCK
            } else {
                StyleFlags::EMPTY
            },
            used_writing_mode: WritingMode::VerticalRl,
            claims: None,
        },
    }
}

fn assert_legacy(recipe: &Recipe, legacy: &LayoutStyle) {
    let prepared = PreparedStyle::prepare(&recipe.values, recipe.metadata);
    let view = TaffyStyle::new(LayoutStyleView::new(
        &prepared,
        ViewContext::default(),
        &[],
        &[],
    ));
    let view = &view;
    macro_rules! same {
        ($t:ident: $($method:ident),+ $(,)?) => { $(assert_eq!($t::$method(view), $t::$method(legacy), stringify!($method));)+ };
    }
    same!(CoreStyle: box_generation_mode, is_block, is_compressible_replaced, is_replaced, box_sizing, direction, overflow, scrollbar_width, position, inset, is_containing_block, size, min_size, max_size, aspect_ratio, margin, padding, border, contain);
    same!(FC: flex_direction, flex_wrap, gap, align_content, align_items, justify_content);
    same!(FI: flex_basis, flex_grow, flex_shrink, align_self);
    same!(GC: grid_auto_flow, gap, align_content, justify_content, align_items, justify_items, grid_template_area_row_count, grid_template_area_column_count);
    same!(GI: grid_row, grid_column, align_self, justify_self);
    assert_eq!(
        GC::grid_auto_columns(view).count(),
        GC::grid_auto_columns(legacy).count()
    );
    assert_eq!(
        GC::grid_auto_rows(view).count(),
        GC::grid_auto_rows(legacy).count()
    );
    assert_eq!(
        GC::grid_template_columns(view).is_some(),
        GC::grid_template_columns(legacy).is_some()
    );
    assert_eq!(
        GC::grid_template_rows(view).is_some(),
        GC::grid_template_rows(legacy).is_some()
    );
}

#[test]
fn icb_recipe_keeps_physical_dimensions_and_independent_direction() {
    for direction in [LayoutDirection::Ltr, LayoutDirection::Rtl] {
        let recipe = initial_containing_block(80.0, 120.0, facts(direction, true));
        let legacy = LayoutStyle {
            display: Display::Block,
            direction,
            size: StyleSize {
                width: Dimension::length(80.0),
                height: Dimension::length(120.0),
            },
            ..LayoutStyle::default()
        };
        assert_eq!(
            recipe.kind,
            RecipeKind::InitialContainingBlock {
                width: 80.0,
                height: 120.0
            }
        );
        assert_eq!(recipe.values.width, L::Px(80.0));
        assert_eq!(recipe.values.height, L::Px(120.0));
        assert_eq!(recipe.metadata.used_writing_mode, WritingMode::VerticalRl);
        assert_legacy(&recipe, &legacy);
    }
}

#[test]
fn contents_and_spanner_have_distinct_origins_with_neutral_layout_values() {
    for direction in [LayoutDirection::Ltr, LayoutDirection::Rtl] {
        let contents = contents(facts(direction, false));
        let spanner = spanner(facts(direction, true));
        assert_eq!(contents.kind, RecipeKind::Contents);
        assert_eq!(spanner.kind, RecipeKind::Spanner);
        let legacy = LayoutStyle {
            display: Display::Block,
            direction,
            ..LayoutStyle::default()
        };
        assert_legacy(
            &contents,
            &LayoutStyle {
                direction,
                ..LayoutStyle::default()
            },
        );
        assert_legacy(&spanner, &legacy);
        assert_eq!(contents.values.width, L::Auto);
        assert_eq!(spanner.values.width, L::Auto);
    }
}

#[test]
fn anonymous_recipes_inherit_only_the_explicit_owner_facts() {
    let inherited = TableFacts {
        owner: 0,
        box_facts: facts(LayoutDirection::Rtl, true),
        border_collapse: BorderCollapseValue::Collapse,
        border_spacing: ComputedBorderSpacing {
            horizontal: raikiri_style::ComputedLength(7.0),
            vertical: raikiri_style::ComputedLength(13.0),
        },
        caption_side: CaptionSideValue::Bottom,
    };
    for recipe in [anonymous_table(inherited), anonymous_cell(inherited)] {
        assert!(matches!(
            recipe.kind,
            RecipeKind::AnonymousTable { .. } | RecipeKind::AnonymousCell { .. }
        ));
        assert_eq!(recipe.values.border_collapse, BorderCollapseValue::Collapse);
        assert_eq!(recipe.values.border_spacing, inherited.border_spacing);
        assert_eq!(recipe.values.caption_side, CaptionSideValue::Bottom);
        assert_eq!(recipe.values.width, L::Auto);
        assert_eq!(recipe.values.margin.left, L::Px(0.0));
        assert_eq!(recipe.metadata.used_writing_mode, WritingMode::VerticalRl);
        assert_legacy(
            &recipe,
            &LayoutStyle {
                display: Display::Block,
                direction: LayoutDirection::Rtl,
                ..LayoutStyle::default()
            },
        );
    }
}

#[test]
fn neutral_source_recipe_matches_native_defaults_and_empty_auto_tracks() {
    let recipe = neutral_source(facts(LayoutDirection::Ltr, false));
    assert_eq!(recipe.kind, RecipeKind::NeutralSource);
    assert_eq!(
        recipe.values.box_sizing,
        raikiri_style::property::BoxSizing::BorderBox
    );
    assert!(recipe.values.grid_auto_columns.is_empty());
    assert!(recipe.values.grid_auto_rows.is_empty());
    assert_legacy(&recipe, &LayoutStyle::default());
}

#[test]
fn neutral_values_share_only_equal_directions() {
    let ltr = neutral_source(facts(LayoutDirection::Ltr, false));
    let rtl = neutral_source(facts(LayoutDirection::Rtl, false));
    let again = neutral_source(facts(LayoutDirection::Rtl, true));
    assert!(std::sync::Arc::ptr_eq(&rtl.values, &again.values));
    assert!(!std::sync::Arc::ptr_eq(&ltr.values, &rtl.values));
    assert_eq!(
        rtl.values.direction,
        raikiri_style::property::Direction::Rtl
    );
    assert_legacy(
        &rtl,
        &LayoutStyle {
            direction: LayoutDirection::Rtl,
            ..LayoutStyle::default()
        },
    );
}

#[test]
fn hidden_box_facts_suppress_box_generation() {
    let mut node = crate::node::Node::new_element("div".into(), None);
    node.set_display(raikiri_style::property::DisplayValue::None);
    let recipe = contents(BoxFacts::for_node(&node, LayoutDirection::Rtl));
    let prepared = PreparedStyle::prepare(&recipe.values, recipe.metadata);
    let view = LayoutStyleView::new(&prepared, ViewContext::default(), &[], &[]);
    assert_eq!(view.box_generation_mode(), BoxGenerationMode::None);
}

#[test]
fn icb_publication_registers_document_owned_recipe_without_extending_source_values() {
    let mut doc = crate::Document::new();
    let html = doc.append_element(
        Some(0),
        "html",
        Some("display:block;direction:rtl;writing-mode:vertical-rl"),
    );
    doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    let mut page = raikiri_traits::PageBox::new();
    page.width = 80.0;
    page.height = 120.0;
    crate::layout::layout_single_page(
        crate::layout::test_support::with_ahem(&mut doc),
        &cascade,
        page,
    )
    .unwrap();
    let record = doc
        .box_style_data
        .get(crate::node::arena::BoxKey::InitialContainingBlock)
        .expect("ICB recipe is published with its node");
    assert_eq!(
        record.recipe,
        Some(RecipeKind::InitialContainingBlock {
            width: 80.0,
            height: 120.0
        })
    );
    let super::super::LayoutBase::Synthetic(values) = &record.base else {
        panic!("ICB owns synthetic values")
    };
    assert_eq!(values.width, L::Px(80.0));
    assert_eq!(values.height, L::Px(120.0));
    assert_eq!(cascade.computed.len(), doc.node_count());
}

#[test]
fn anonymous_publication_uses_the_lowering_owner_for_each_recipe() {
    let mut doc = crate::Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    let owner = doc.append_element(Some(body), "div", Some("display:block;direction:rtl;border-collapse:collapse;border-spacing:7px 13px;caption-side:bottom;width:300px"));
    let row = doc.append_element(Some(owner), "div", Some("display:table-row;direction:ltr;border-collapse:separate;border-spacing:17px 19px;caption-side:top;width:777px"));
    doc.append_text(row, "A");
    let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    crate::layout::layout_single_page(
        crate::layout::test_support::with_ahem(&mut doc),
        &cascade,
        crate::layout::test_support::page_box_800x600(),
    )
    .unwrap();
    let table = doc.nodes.anonymous_table_key(0);
    let cell = doc.nodes.anonymous_cell_key(0);
    for (key, record_key, source, kind) in [
        (
            table,
            crate::node::arena::BoxKey::AnonymousTable(0),
            owner,
            RecipeKind::AnonymousTable { owner },
        ),
        (
            cell,
            crate::node::arena::BoxKey::AnonymousCell(0),
            row,
            RecipeKind::AnonymousCell { owner: row },
        ),
    ] {
        assert_eq!(doc.ifc_source_owner(key), source);
        let record = doc
            .box_style_data
            .get(record_key)
            .expect("anonymous recipe is published after lowering");
        assert_eq!(record.recipe, Some(kind));
        let super::super::LayoutBase::Synthetic(values) = &record.base else {
            panic!("anonymous box owns synthetic values")
        };
        assert_eq!(
            values.border_collapse,
            cascade.computed[source].border_collapse
        );
        assert_eq!(
            values.border_spacing,
            cascade.computed[source].border_spacing
        );
        assert_eq!(values.caption_side, cascade.computed[source].caption_side);
        assert_eq!(
            record.metadata.used_writing_mode,
            doc.nodes[source].used_writing_mode
        );
        assert_eq!(values.width, L::Auto);
    }
    assert_eq!(cascade.computed.len(), doc.node_count());
}

#[test]
fn contents_and_lifted_spanner_publish_distinct_source_recipe_origins() {
    for lifted in [false, true] {
        let mut doc = crate::Document::new();
        let html = doc.append_element(Some(0), "html", Some("display:block"));
        let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
        let parent = if lifted {
            doc.append_element(
                Some(body),
                "div",
                Some("display:block;column-count:2;width:120px;direction:rtl"),
            )
        } else {
            body
        };
        let css = if lifted {
            "display:block"
        } else {
            "display:contents;direction:rtl;position:absolute;width:777px;contain:size"
        };
        let source = doc.append_element(Some(parent), "div", Some(css));
        doc.append_element(
            Some(source),
            "div",
            Some("display:block;column-span:all;height:20px"),
        );
        let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
        crate::layout::layout_single_page(
            crate::layout::test_support::with_ahem(&mut doc),
            &cascade,
            crate::layout::test_support::page_box_800x600(),
        )
        .unwrap();
        assert_eq!(
            doc.nodes[source].display(),
            raikiri_style::property::DisplayValue::Contents
        );
        assert_eq!(
            cascade.computed[source].display,
            if lifted {
                raikiri_style::property::DisplayValue::Block
            } else {
                raikiri_style::property::DisplayValue::Contents
            }
        );
        let record = doc
            .box_style_data
            .get(crate::node::arena::BoxKey::Source(source))
            .expect("neutralized source publishes its recipe");
        let kind = if lifted {
            RecipeKind::Spanner
        } else {
            RecipeKind::Contents
        };
        assert_eq!(record.recipe, Some(kind));
        let super::super::LayoutBase::Synthetic(values) = &record.base else {
            panic!("neutralized source owns synthetic values")
        };
        assert_eq!(values.width, L::Auto);
        assert_eq!(cascade.computed.len(), doc.node_count());
    }
}

#[test]
fn neutral_source_publication_covers_document_text_comment_and_fragment() {
    let mut doc = crate::Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;direction:rtl"),
    );
    let text = doc.append_text(body, "A");
    let comment = doc.append_comment(Some(body), "comment");
    let fragment = doc.create_detached_fragment();
    let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    crate::layout::layout_single_page(
        crate::layout::test_support::with_ahem(&mut doc),
        &cascade,
        crate::layout::test_support::page_box_800x600(),
    )
    .unwrap();
    for source in [0, text, comment, fragment] {
        let record = doc
            .box_style_data
            .get(crate::node::arena::BoxKey::Source(source))
            .expect("every non-element source has neutral recipe data");
        assert_eq!(record.recipe, Some(RecipeKind::NeutralSource));
        assert!(
            matches!(&record.base, super::super::LayoutBase::Synthetic(_)),
            "neutral source owns synthetic values"
        );
    }
    assert_eq!(cascade.computed.len(), doc.node_count());
}

#[test]
fn icb_dimensions_republish_without_reusing_an_old_pass() {
    let mut doc = crate::Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    let mut page = raikiri_traits::PageBox::new();
    page.width = 80.0;
    page.height = 120.0;
    crate::layout::layout_single_page(
        crate::layout::test_support::with_ahem(&mut doc),
        &cascade,
        page,
    )
    .unwrap();
    let key = crate::node::arena::BoxKey::InitialContainingBlock;
    let old = doc.box_style_data.get(key).unwrap().stamp.recipe;
    doc.box_style_data.get_mut(key).unwrap().pass.push(
        raikiri_layout_style::OwnedPropertyOverride::aspect_ratio(Some(2.0)),
    );
    page.width = 160.0;
    page.height = 240.0;
    crate::layout::layout_single_page(&mut doc, &cascade, page).unwrap();
    let current = doc.box_style_data.get(key).unwrap();
    assert_ne!(current.stamp.recipe, old);
    assert!(current.pass.is_empty());
    let super::super::LayoutBase::Synthetic(values) = &current.base else {
        panic!("ICB owns synthetic values")
    };
    assert_eq!(values.width, L::Px(160.0));
    assert_eq!(values.height, L::Px(240.0));
    assert_eq!(cascade.computed.len(), doc.node_count());
}

#[test]
fn restoring_a_source_box_discards_contents_and_spanner_recipes() {
    for lifted in [false, true] {
        let mut doc = crate::Document::new();
        let html = doc.append_element(Some(0), "html", Some("display:block"));
        let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
        let parent = doc.append_element(
            Some(body),
            "div",
            Some(if lifted {
                "display:block;column-count:2;width:120px"
            } else {
                "display:block"
            }),
        );
        let source = doc.append_element(
            Some(parent),
            "div",
            Some(if lifted {
                "display:block"
            } else {
                "display:contents;width:777px"
            }),
        );
        doc.append_element(
            Some(source),
            "div",
            Some("display:block;column-span:all;height:20px"),
        );
        let first = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
        crate::layout::layout_single_page(
            crate::layout::test_support::with_ahem(&mut doc),
            &first,
            crate::layout::test_support::page_box_800x600(),
        )
        .unwrap();
        let key = crate::node::arena::BoxKey::Source(source);
        assert!(doc.box_style_data.get(key).unwrap().recipe.is_some());
        doc.box_style_data.get_mut(key).unwrap().pass.push(
            raikiri_layout_style::OwnedPropertyOverride::aspect_ratio(Some(2.0)),
        );
        if lifted {
            doc.set_element_inline_style(parent, Some("display:block;width:120px".into()));
        } else {
            doc.set_element_inline_style(source, Some("display:block;width:40px".into()));
        }
        let second = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
        crate::layout::layout_single_page(
            &mut doc,
            &second,
            crate::layout::test_support::page_box_800x600(),
        )
        .unwrap();
        assert_eq!(
            doc.nodes[source].display(),
            raikiri_style::property::DisplayValue::Block
        );
        {
            let record = doc
                .box_style_data
                .get(key)
                .expect("ordinary source foundation is restored");
            assert!(record.recipe.is_none());
            // A projected block width is valid in the new pass, but the old ratio is not.
            assert_eq!(
                crate::layout::test_support::with_pass_view(
                    &doc,
                    key,
                    &raikiri_style::ComputedValues::initial(),
                    |view| view.aspect_ratio(),
                ),
                None
            );
            assert!(matches!(record.base, super::super::LayoutBase::Source(id) if id == source));
        }
        assert_eq!(second.computed.len(), doc.node_count());
    }
}
