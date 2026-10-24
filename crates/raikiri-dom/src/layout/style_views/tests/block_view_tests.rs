use super::*;
use raikiri_layout_geometry::Point;

#[test]
fn block_view_prepares_normal_source_and_text_keys() {
    let (mut doc, cascade, child, text) =
        fixture_with_styles("display:block", "width:31px;height:7px");
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
    for idx in tree.nodes.layout_keys() {
        assert!(tree.has_style_view(idx), "missing all-key view for {idx}");
    }
    assert_eq!(
        tree.style_view(child).size().width,
        taffy::Dimension::length(31.0)
    );
    assert_eq!(tree.style_view(text).size(), LayoutStyle::default().size());
}

use crate::layout::style_views::ReadRole;
use raikiri_layout_style::LayoutStyleView;
use taffy::{BlockContainerStyle as BC, BlockItemStyle as BI, OofItemStyle as OI};

fn read_core(query: &LayoutStyleView<'_>) {
    macro_rules! read {
        ($($method:ident),+ $(,)?) => { $(let _ = taffy::CoreStyle::$method(query);)+ };
    }
    read!(
        box_generation_mode,
        is_block,
        is_compressible_replaced,
        is_replaced,
        box_sizing,
        direction,
        overflow,
        scrollbar_width,
        position,
        inset,
        is_containing_block,
        size,
        min_size,
        max_size,
        aspect_ratio,
        margin,
        padding,
        border,
        contain
    );
}

#[test]
fn block_view_reads_each_new_role_and_bridge_alignment() {
    let (mut doc, cascade, child, _) = fixture_with_styles(
        "display:block;width:100px",
        "display:block;width:calc(21px + 10%);height:7px;padding:10%;border:2px solid;transform:translateX(0);align-content:space-between;align-self:center;justify-self:end;clear:both",
    );
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
    for role in [
        ReadRole::CoreContainer,
        ReadRole::BlockContainer,
        ReadRole::BlockItem,
        ReadRole::Oof,
        ReadRole::Physical,
    ] {
        let query = tree.query_style(child, role);
        read_core(&query);
        match role {
            ReadRole::CoreContainer => assert!(query.is_containing_block().fixed),
            ReadRole::BlockContainer => {
                assert!(!query.is_containing_block().fixed);
                let _ = (
                    BC::text_align(&query),
                    BC::align_content(&query),
                    BC::justify_items(&query),
                );
            }
            ReadRole::BlockItem => {
                assert!(!query.is_containing_block().fixed);
                assert_eq!(BI::align_self(&query), None);
                assert_eq!(BI::justify_self(&query), None);
                assert_eq!(BI::align_content(&query), taffy::AlignContent::NORMAL);
                let _ = (BI::is_table(&query), BI::float(&query), BI::clear(&query));
            }
            ReadRole::Physical => {
                assert!(!query.is_containing_block().fixed);
                let _ = (BI::float(&query), BI::clear(&query));
            }
            ReadRole::Oof => {
                assert!(!query.is_containing_block().fixed);
                let _ = (
                    OI::align_self(&query),
                    OI::justify_self(&query),
                    OI::is_table(&query),
                    OI::grid_row(&query),
                    OI::grid_column(&query),
                );
            }
            _ => unreachable!(),
        }
        assert_eq!(query.bridge_align_self(), Some(taffy::AlignSelf::CENTER));
        assert_eq!(query.bridge_justify_self(), Some(taffy::AlignSelf::END));
        assert_eq!(
            query.bridge_align_content(),
            taffy::AlignContent::SPACE_BETWEEN
        );
    }
}

#[test]
fn block_view_basis_is_role_specific_and_scopes_restore() {
    let (mut doc, cascade, child, _) =
        fixture_with_styles("display:block;width:100px", "width:31px;padding:10%");
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
    tree.context
        .block_item_percent_bases
        .push((parent, Some(100.0)));
    let item = tree.query_style(child, ReadRole::BlockItem);
    assert_eq!(item.padding().top, taffy::LengthPercentage::length(10.0));
    assert_eq!(item.padding().left, taffy::LengthPercentage::percent(0.1));
    assert_eq!(
        tree.query_style(child, ReadRole::Oof).padding().top,
        taffy::LengthPercentage::percent(0.1)
    );
    let key = tree.nodes.box_key(child).unwrap();
    tree.with_overrides(
        vec![(
            key,
            OwnedPropertyOverride::length(LengthProperty::Width, L::Px(19.0), true).unwrap(),
        )],
        |tree| {
            for role in [
                ReadRole::CoreContainer,
                ReadRole::BlockContainer,
                ReadRole::BlockItem,
                ReadRole::Oof,
            ] {
                assert_eq!(
                    tree.query_style(child, role).size().width,
                    taffy::Dimension::length(19.0)
                );
            }
        },
    );
    assert_eq!(
        tree.query_style(child, ReadRole::BlockItem).size().width,
        taffy::Dimension::length(31.0)
    );
    tree.context.block_item_percent_bases.pop();
    assert_eq!(
        tree.query_style(child, ReadRole::BlockItem).padding().top,
        taffy::LengthPercentage::percent(0.1)
    );
}

#[test]
fn block_view_oof_table_identity_is_independent() {
    for display in ["table", "inline-table", "block"] {
        let (mut doc, cascade, child, _) =
            fixture_with_styles("display:block", &format!("display:{display};width:31px"));
        let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
        apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
        assert!(!BI::is_table(&tree.query_style(child, ReadRole::BlockItem)));
        assert_eq!(
            OI::is_table(&tree.query_style(child, ReadRole::Oof)),
            display != "block"
        );
        assert_eq!(
            tree.query_style(child, ReadRole::BlockItem).size().width,
            taffy::Dimension::length(31.0)
        );
    }
    let (mut doc, cascade, child, _) = fixture_with_styles("display:block", "width:31px");
    let tree = LayoutSession::with_cascade(&mut doc, &cascade);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tree.query_style(child, ReadRole::BlockItem).size()
    }));
    assert!(result.is_err());
}

#[test]
fn block_view_materialized_input_reads_real_getters_and_bridge_alignment() {
    use crate::layout::dispatch::block_flow::style_input;
    let (mut doc, cascade, child, _) = fixture_with_styles(
        "display:block;width:100px",
        "display:block;width:31px;height:7px;align-self:center;justify-self:end;align-content:space-between;float:left;clear:both",
    );
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
    let container = style_input::container(&tree, parent);
    assert_eq!(container.size.width, taffy::Dimension::length(100.0));
    let input = style_input::child(&tree, child);
    assert_eq!(
        input.size,
        StyleSize {
            width: taffy::Dimension::length(31.0),
            height: taffy::Dimension::length(7.0)
        }
    );
    assert!(input.is_block());
    assert_eq!(input.align_self, Some(taffy::AlignSelf::CENTER));
    assert_eq!(input.justify_self, Some(taffy::AlignSelf::END));
    assert_eq!(input.align_content, taffy::AlignContent::SPACE_BETWEEN);
    assert_eq!(input.float, taffy::Float::Left);
    assert_eq!(input.clear, taffy::Clear::Both);
}

#[test]
fn block_view_basis_excludes_absolute_items_other_parents_and_unknown_widths() {
    for position in ["static", "absolute"] {
        let (mut doc, cascade, child, _) = fixture_with_styles(
            "display:block",
            &format!("display:block;position:{position};padding:10%"),
        );
        let parent = doc.nodes[child].parent.unwrap();
        let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
        apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
        for (owner, basis) in [(parent, None), (child, Some(100.0))] {
            tree.context.block_item_percent_bases.push((owner, basis));
            assert_eq!(
                tree.query_style(child, ReadRole::BlockItem).padding().top,
                taffy::LengthPercentage::percent(0.1)
            );
            tree.context.block_item_percent_bases.pop();
        }
        tree.context
            .block_item_percent_bases
            .push((parent, Some(100.0)));
        let expected = if position == "absolute" {
            taffy::LengthPercentage::percent(0.1)
        } else {
            taffy::LengthPercentage::length(10.0)
        };
        assert_eq!(
            tree.query_style(child, ReadRole::BlockItem).padding().top,
            expected
        );
    }
}

pub(super) fn assert_all_new_roles(tree: &LayoutSession<'_>, idx: usize) {
    for role in [
        ReadRole::CoreContainer,
        ReadRole::BlockContainer,
        ReadRole::BlockItem,
        ReadRole::Oof,
    ] {
        let query = tree.query_style(idx, role);
        read_core(&query);
        match role {
            ReadRole::BlockContainer => {
                let _ = (
                    BC::text_align(&query),
                    BC::align_content(&query),
                    BC::justify_items(&query),
                );
            }
            ReadRole::BlockItem => {
                let _ = (
                    BI::is_table(&query),
                    BI::align_self(&query),
                    BI::justify_self(&query),
                    BI::align_content(&query),
                    BI::float(&query),
                    BI::clear(&query),
                );
            }
            ReadRole::Oof => {
                let _ = (
                    OI::align_self(&query),
                    OI::justify_self(&query),
                    OI::is_table(&query),
                    OI::grid_row(&query),
                    OI::grid_column(&query),
                );
            }
            _ => {}
        }
        let _ = (
            query.bridge_align_self(),
            query.bridge_justify_self(),
            query.bridge_align_content(),
        );
    }
}

#[test]
fn native_sticky_flex_grid_insets_keep_the_bridge_position() {
    for display in ["flex", "grid"] {
        let (mut doc, cascade, child, _) = fixture_with_styles(
            &format!("display:{display};width:100px"),
            "display:block;position:sticky;left:13px;top:17px;width:10px;height:10px",
        );
        crate::layout::test_support::with_ahem(&mut doc);
        crate::layout::page_pipeline::layout_single_page_checked(
            &mut doc,
            &cascade,
            crate::layout::test_support::page_box_800x600(),
        )
        .unwrap();
        assert_eq!(doc.nodes[child].layout.location, Point::ZERO);
        let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
        tree.prepare_style_views();
        for role in [
            ReadRole::FlexContainer,
            ReadRole::FlexItem,
            ReadRole::GridContainer,
            ReadRole::GridItem,
            ReadRole::CoreContainer,
            ReadRole::BlockContainer,
            ReadRole::BlockItem,
            ReadRole::Oof,
        ] {
            assert_eq!(
                tree.query_style(child, role).position(),
                taffy::Position::Sticky
            );
        }
    }
}
