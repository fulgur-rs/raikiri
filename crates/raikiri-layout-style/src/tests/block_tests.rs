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
    assert_eq!(BI::align_self(&block), None);
    assert_eq!(BI::justify_self(&block), None);
    assert_eq!(BI::align_content(&block), taffy::AlignContent::NORMAL);
    assert_eq!(block.bridge_align_self(), Some(taffy::AlignSelf::CENTER));
    assert_eq!(block.bridge_justify_self(), Some(taffy::AlignSelf::END));
    assert_eq!(
        block.bridge_align_content(),
        taffy::AlignContent::SPACE_BETWEEN
    );
    assert_eq!(
        BC::align_content(&block),
        taffy::AlignContent::SPACE_BETWEEN
    );
    assert_eq!(BC::text_align(&block), taffy::TextAlign::Auto);
    assert_eq!(BC::justify_items(&block), taffy::AlignItems::NORMAL);
    assert_eq!(BI::float(&block), taffy::Float::Left);
    assert_eq!(OI::align_self(&block), Some(taffy::AlignSelf::CENTER));
    assert_eq!(OI::justify_self(&block), Some(taffy::AlignSelf::END));
    assert_eq!(OI::grid_row(&block), taffy::GridItemStyle::grid_row(&block));
    assert_eq!(
        OI::grid_column(&block),
        taffy::GridItemStyle::grid_column(&block)
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
        assert_eq!(BI::is_table(&view), block);
        assert_eq!(OI::is_table(&view), oof);
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
        assert_eq!(BI::float(&view), expected);
        assert_eq!(BI::clear(&view), taffy::Clear::Both);
    }
}
