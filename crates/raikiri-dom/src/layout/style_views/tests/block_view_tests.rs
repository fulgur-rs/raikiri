use super::*;

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
    assert_eq!(
        tree.style_view(text).size(),
        tree.layout_style_for_key(text).unwrap().size()
    );
}

use crate::layout::style_views::checked::{CheckedStyle, ReadRole};
use taffy::{BlockContainerStyle as BC, BlockItemStyle as BI, OofItemStyle as OI};

fn read_core(query: &CheckedStyle<'_>) {
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
fn block_view_compares_each_new_role_and_bridge_alignment_at_read() {
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
        let query = tree.checked_query(child, role);
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
    for count in &tree.style_view_comparisons.getter_reads[4..ReadRole::Pagination as usize] {
        assert!(count.get() > 0);
    }
    assert!(tree.style_view_comparisons.calc_comparisons.get() > 0);
    for name in [
        "Bridge::align_self",
        "Bridge::justify_self",
        "Bridge::align_content",
    ] {
        assert!(
            tree.style_view_comparisons
                .properties
                .borrow()
                .get(name)
                .copied()
                .unwrap_or(0)
                > 0
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
    let item = tree.checked_query(child, ReadRole::BlockItem);
    assert_eq!(item.padding().top, taffy::LengthPercentage::length(10.0));
    assert_eq!(item.padding().left, taffy::LengthPercentage::percent(0.1));
    assert_eq!(
        tree.checked_query(child, ReadRole::Oof).padding().top,
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
                    tree.checked_query(child, role).size().width,
                    taffy::Dimension::length(19.0)
                );
            }
        },
    );
    assert_eq!(
        tree.checked_query(child, ReadRole::BlockItem).size().width,
        taffy::Dimension::length(31.0)
    );
    assert!(tree.style_view_comparisons.scoped_reads.get() > 0);
    assert!(tree.style_view_comparisons.restored_reads.get() == 0);
    tree.context.block_item_percent_bases.pop();
    let _ = tree.checked_query(child, ReadRole::BlockItem).size();
    assert!(tree.style_view_comparisons.restored_reads.get() > 0);
}

#[test]
fn block_view_oof_table_identity_and_raw_session_are_independent() {
    for display in ["table", "inline-table", "block"] {
        let (mut doc, cascade, child, _) =
            fixture_with_styles("display:block", &format!("display:{display};width:31px"));
        let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
        apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
        assert!(!BI::is_table(
            &tree.checked_query(child, ReadRole::BlockItem)
        ));
        assert_eq!(
            OI::is_table(&tree.checked_query(child, ReadRole::Oof)),
            display != "block"
        );
        drop(tree);
        let tree = LayoutSession::new(&mut doc);
        let query = tree.checked_query(child, ReadRole::BlockItem);
        assert_eq!(query.size().width, taffy::Dimension::length(31.0));
    }
    let (mut doc, cascade, child, _) = fixture_with_styles("display:block", "width:31px");
    let tree = LayoutSession::with_cascade(&mut doc, &cascade);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tree.checked_query(child, ReadRole::BlockItem).size()
    }));
    assert!(result.is_err());
}

#[test]
fn block_view_materialized_input_reads_real_getters_and_raw_alignment() {
    use crate::taffy_impl::block_flow::style_input;
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
        taffy::Size {
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
    assert!(tree.style_view_comparisons.getter_reads[ReadRole::BlockContainer as usize].get() > 0);
    assert!(tree.style_view_comparisons.getter_reads[ReadRole::BlockItem as usize].get() > 0);
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
                tree.checked_query(child, ReadRole::BlockItem).padding().top,
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
            tree.checked_query(child, ReadRole::BlockItem).padding().top,
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
        let query = tree.checked_query(idx, role);
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
fn block_view_preserves_each_box_scrollbar_input_across_preparation() {
    let (mut doc, cascade, child, text) = fixture_with_styles("display:block", "display:block");
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
    for (idx, width) in [(child, 20.0), (text, 13.0)] {
        tree.layout_style_for_key_mut(idx).unwrap().scrollbar_width = width;
    }
    for _ in 0..2 {
        tree.prepare_style_views();
        for (idx, width) in [(child, 20.0), (text, 13.0)] {
            assert_eq!(tree.style_view(idx).scrollbar_width(), width);
            assert_eq!(
                tree.checked_query(idx, ReadRole::BlockItem)
                    .scrollbar_width(),
                width
            );
        }
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
        assert_eq!(
            doc.nodes[child].unrounded_layout.location,
            taffy::Point::ZERO
        );
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
                tree.checked_query(child, role).position(),
                taffy::Position::Sticky
            );
        }
    }
}

#[test]
fn block_view_keeps_programmatic_table_identity_distinct_from_css_tables() {
    let (mut doc, cascade, child, text) = fixture_with_styles("display:block", "display:block");
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
    for table in [true, false] {
        for idx in [child, text] {
            tree.layout_style_for_key_mut(idx).unwrap().item_is_table = table;
        }
        tree.prepare_style_views();
        for idx in [child, text] {
            assert_eq!(
                BI::is_table(&tree.checked_query(idx, ReadRole::BlockItem)),
                table
            );
            assert_eq!(OI::is_table(&tree.checked_query(idx, ReadRole::Oof)), table);
        }
    }
}

#[test]
fn block_view_preserves_constructor_text_alignment_and_contents_neutralization() {
    for (display, expected) in [
        ("block", taffy::TextAlign::LegacyRight),
        ("contents", taffy::TextAlign::Auto),
    ] {
        let mut doc = Document::new();
        let root = doc.append_element(
            Some(0),
            "html",
            LayoutStyle::default(),
            Some("display:block"),
        );
        let child = doc.append_element(
            Some(root),
            "div",
            LayoutStyle {
                text_align: taffy::TextAlign::LegacyRight,
                ..LayoutStyle::default()
            },
            Some(format!("display:{display};text-align:center")),
        );
        doc.mark_in_document_flags();
        let cascade = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
        apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
        for _ in 0..2 {
            assert_eq!(
                BC::text_align(&tree.checked_query(child, ReadRole::BlockContainer)),
                expected
            );
            tree.prepare_style_views();
        }
    }
}

#[test]
fn block_view_preserves_constructor_compressible_replaced_supplement() {
    let mut doc = Document::new();
    let child = doc.append_element(
        Some(0),
        "div",
        LayoutStyle {
            item_is_compressible_replaced: true,
            ..LayoutStyle::default()
        },
        Some("display:block;width:31px;height:7px"),
    );
    doc.mark_in_document_flags();
    let cascade = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
    for compressible in [true, false] {
        tree.layout_style_for_key_mut(child)
            .unwrap()
            .item_is_compressible_replaced = compressible;
        tree.prepare_style_views();
        for role in [ReadRole::CoreContainer, ReadRole::BlockItem, ReadRole::Oof] {
            let query = tree.checked_query(child, role);
            assert!(!query.is_replaced());
            assert_eq!(query.is_compressible_replaced(), compressible);
        }
    }
}
