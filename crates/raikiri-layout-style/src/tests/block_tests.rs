use super::*;
use raikiri_style::property::{
    AlignSelfValue, ContentAlignmentValue, FloatValue, SelfAlignmentValue,
};
use taffy::{BlockContainerStyle as BC, BlockItemStyle as BI, OofItemStyle as OI};

#[test]
fn core_claims_and_block_defaults_stay_distinct() {
    let mut cv = ComputedValues::initial();
    cv.align_self = AlignSelfValue::Value(SelfAlignmentValue::Center);
    cv.justify_self = AlignSelfValue::Value(SelfAlignmentValue::End);
    cv.align_content = ContentAlignmentValue::SpaceBetween;
    cv.float = FloatValue::Left;
    let metadata = StyleMetadata {
        claims: Some(taffy::ContainingBlockClaims {
            absolute: true,
            fixed: true,
        }),
        ..StyleMetadata::default()
    };
    let prepared = PreparedStyle::prepare(&cv, metadata);
    let view = LayoutStyleView::new(&prepared, ViewContext::default(), &[], &[]);
    assert!(view.is_containing_block().fixed);
    let block = view.with_context(ViewContext {
        claims_policy: ClaimsPolicy::Positioning,
        ..ViewContext::default()
    });
    assert!(!block.is_containing_block().absolute);
    assert!(!block.is_containing_block().fixed);
    let projected = TaffyStyle::new(block);
    assert_eq!(BI::align_self(&projected), None);
    assert_eq!(BI::justify_self(&projected), None);
    assert_eq!(BI::align_content(&projected), taffy::AlignContent::NORMAL);
    assert_eq!(block.bridge_align_self(), Some(taffy::AlignSelf::CENTER));
    assert_eq!(block.bridge_justify_self(), Some(taffy::AlignSelf::END));
    assert_eq!(
        block.bridge_align_content(),
        taffy::AlignContent::SPACE_BETWEEN
    );
    assert_eq!(
        BC::align_content(&projected),
        taffy::AlignContent::SPACE_BETWEEN
    );
    assert_eq!(BC::text_align(&projected), taffy::TextAlign::Auto);
    assert_eq!(BC::justify_items(&projected), taffy::AlignItems::NORMAL);
    assert_eq!(BI::float(&projected), taffy::Float::Left);
    assert_eq!(OI::align_self(&projected), Some(taffy::AlignSelf::CENTER));
    assert_eq!(OI::justify_self(&projected), Some(taffy::AlignSelf::END));
    assert_eq!(
        OI::grid_row(&projected),
        taffy::GridItemStyle::grid_row(&projected)
    );
    assert_eq!(
        OI::grid_column(&projected),
        taffy::GridItemStyle::grid_column(&projected)
    );
}

#[test]
fn block_and_oof_table_facts_are_separate_from_formatting_context() {
    let cv = ComputedValues::initial();
    for (flags, block, oof) in [
        (StyleFlags::TABLE, false, false),
        (StyleFlags::BLOCK_TABLE, true, true),
        (StyleFlags::OOF_TABLE, false, true),
    ] {
        let prepared = PreparedStyle::prepare(
            &cv,
            StyleMetadata {
                flags,
                ..StyleMetadata::default()
            },
        );
        let view = LayoutStyleView::new(&prepared, ViewContext::default(), &[], &[]);
        assert_eq!(view.is_table(), block);
        assert_eq!(BI::is_table(&TaffyStyle::new(view)), block);
        assert_eq!(OI::is_table(&TaffyStyle::new(view)), oof);
    }
}

#[test]
fn block_float_and_clear_keep_authored_and_override_values() {
    use raikiri_style::property::ClearValue;
    for (float, expected) in [
        (FloatValue::None, taffy::Float::None),
        (FloatValue::Left, taffy::Float::Left),
        (FloatValue::Right, taffy::Float::Right),
    ] {
        let mut cv = ComputedValues::initial();
        cv.float = float;
        cv.clear = ClearValue::Left;
        let prepared = PreparedStyle::prepare(&cv, StyleMetadata::default());
        let pass = [OwnedPropertyOverride::clear(ClearValue::Both)];
        let view =
            LayoutStyleView::with_owned_overrides(&prepared, ViewContext::default(), &pass, &[]);
        assert_eq!(view.float(), expected);
        assert_eq!(view.clear(), taffy::Clear::Both);
        assert_eq!(BI::float(&TaffyStyle::new(view)), expected);
        assert_eq!(BI::clear(&TaffyStyle::new(view)), taffy::Clear::Both);
    }
}

#[test]
fn sticky_position_and_source_vertical_minimum_preserve_bridge_policy() {
    use raikiri_style::ComputedLengthPercentageOrAuto as L;
    use raikiri_style::property::{PositionValue, WritingMode};
    let mut cv = ComputedValues::initial();
    cv.position = PositionValue::Sticky;
    let prepared = PreparedStyle::prepare(&cv, StyleMetadata::default());
    let view = LayoutStyleView::new(&prepared, ViewContext::default(), &[], &[]);
    assert_eq!(view.position(), taffy::Position::Sticky);
    cv.cssom_writing_mode = WritingMode::VerticalRl;
    cv.min_height = L::Auto;
    cv.min_block_size = Some(L::Px(64.0));
    for (flags, expected) in [
        (StyleFlags::BLOCK, taffy::LengthPercentageAuto::length(64.0)),
        (
            StyleFlags::BLOCK.union(StyleFlags::SOURCE_VERTICAL_DIRECTION),
            taffy::LengthPercentageAuto::auto(),
        ),
    ] {
        let prepared = PreparedStyle::prepare(
            &cv,
            StyleMetadata {
                flags,
                ..StyleMetadata::default()
            },
        );
        let view = LayoutStyleView::new(&prepared, ViewContext::default(), &[], &[]);
        assert_eq!(view.min_size().height, expected);
    }
}
