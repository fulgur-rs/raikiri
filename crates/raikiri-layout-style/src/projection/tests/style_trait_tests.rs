//! What a projected view answers through the library's style traits, set
//! against what the view itself answers.

use super::*;
use raikiri_style::ComputedLengthPercentageOrAuto as L;
use raikiri_style::property::*;
use taffy::{
    BlockContainerStyle as BC, BlockItemStyle as BI, CoreStyle as CS, FlexboxContainerStyle as FC,
    FlexboxItemStyle as FI, GridItemStyle as GI, OofItemStyle as OI,
};

fn query(cv: &ComputedValues, f: impl FnOnce(LayoutStyleView<'_>)) {
    let p = PreparedStyle::new(cv, StyleMetadata::default(), &[]).unwrap();
    f(LayoutStyleView::new(&p, ViewContext::default(), &[], &[]));
}

#[test]
fn flex_direction_and_wrap_keywords_have_explicit_results() {
    for (input, expected) in [
        (FlexDirectionValue::Row, taffy::FlexDirection::Row),
        (
            FlexDirectionValue::RowReverse,
            taffy::FlexDirection::RowReverse,
        ),
        (FlexDirectionValue::Column, taffy::FlexDirection::Column),
        (
            FlexDirectionValue::ColumnReverse,
            taffy::FlexDirection::ColumnReverse,
        ),
    ] {
        let mut cv = ComputedValues::initial();
        cv.flex_direction = input;
        query(&cv, |v| {
            assert_eq!(v.flex_direction(), expected);
            assert_eq!(FC::flex_direction(&TaffyStyle::new(v)), expected);
        });
    }
    for (input, expected) in [
        (FlexWrapValue::NoWrap, taffy::FlexWrap::NoWrap),
        (FlexWrapValue::Wrap, taffy::FlexWrap::Wrap),
        (FlexWrapValue::WrapReverse, taffy::FlexWrap::WrapReverse),
    ] {
        let mut cv = ComputedValues::initial();
        cv.flex_wrap = input;
        query(&cv, |v| {
            assert_eq!(v.flex_wrap(), expected);
            assert_eq!(FC::flex_wrap(&TaffyStyle::new(v)), expected);
        });
    }
}

#[test]
fn explicit_normal_alignment_is_projected_as_normal() {
    let mut cv = ComputedValues::initial();
    cv.align_self = AlignSelfValue::Value(SelfAlignmentValue::Normal);
    cv.justify_self = cv.align_self;
    query(&cv, |v| {
        let projected = TaffyStyle::new(v);
        assert_eq!(FI::align_self(&projected), Some(taffy::AlignItems::NORMAL));
        assert_eq!(GI::align_self(&projected), Some(taffy::AlignItems::NORMAL));
        assert_eq!(
            GI::justify_self(&projected),
            Some(taffy::AlignItems::NORMAL)
        );
    });
}

#[test]
fn core_claims_and_block_defaults_are_projected_apart() {
    let mut cv = ComputedValues::initial();
    cv.align_self = AlignSelfValue::Value(SelfAlignmentValue::Center);
    cv.justify_self = AlignSelfValue::Value(SelfAlignmentValue::End);
    cv.align_content = ContentAlignmentValue::SpaceBetween;
    cv.float = FloatValue::Left;
    let metadata = StyleMetadata {
        claims: Some(values::ContainingBlockClaims {
            absolute: true,
            fixed: true,
        }),
        ..StyleMetadata::default()
    };
    let prepared = PreparedStyle::prepare(&cv, metadata);
    let view = LayoutStyleView::new(&prepared, ViewContext::default(), &[], &[]);
    let block = view.with_context(ViewContext {
        claims_policy: ClaimsPolicy::Positioning,
        ..ViewContext::default()
    });
    let projected = TaffyStyle::new(block);
    assert_eq!(BI::align_self(&projected), None);
    assert_eq!(BI::justify_self(&projected), None);
    assert_eq!(BI::align_content(&projected), taffy::AlignContent::NORMAL);
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
fn block_and_oof_table_facts_are_projected_separately() {
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
        assert_eq!(BI::is_table(&TaffyStyle::new(view)), block);
        assert_eq!(OI::is_table(&TaffyStyle::new(view)), oof);
    }
}

#[test]
fn block_float_and_clear_are_projected_with_their_overrides() {
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
        assert_eq!(BI::float(&TaffyStyle::new(view)), expected);
        assert_eq!(BI::clear(&TaffyStyle::new(view)), taffy::Clear::Both);
    }
}

#[test]
fn owned_and_borrowed_projections_agree() {
    let mut cv = ComputedValues::initial();
    let expression = CalcLengthPercentage {
        px: -10.0,
        percent: 25.0,
    };
    cv.width = L::Calc(expression);
    cv.margin.left = L::Calc(expression);
    let positive = AlignedCalc::new(expression, true);
    let signed = AlignedCalc::new(expression, false);
    let bindings = [
        PreparedCalc {
            slot: CalcSlot::Property(LengthProperty::Width),
            payload: &positive,
        },
        PreparedCalc {
            slot: CalcSlot::Property(LengthProperty::MarginLeft),
            payload: &signed,
        },
    ];
    let borrowed = PreparedStyle::new(&cv, StyleMetadata::default(), &bindings).unwrap();
    let owned = PreparedStyle::prepare(&cv, StyleMetadata::default());
    let a = LayoutStyleView::new(&borrowed, ViewContext::default(), &[], &[]);
    let b = LayoutStyleView::new(&owned, ViewContext::default(), &[], &[]);
    macro_rules! same {
        ($trait:ident, $($getter:ident),+ $(,)?) => { $(
            assert_eq!(
                $trait::$getter(&TaffyStyle::new(a)),
                $trait::$getter(&TaffyStyle::new(b)),
                stringify!($getter)
            );
        )+ };
    }
    same!(
        CS,
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
        min_size,
        max_size,
        aspect_ratio,
        padding,
        border,
        contain
    );
    same!(
        FC,
        flex_direction,
        flex_wrap,
        gap,
        align_content,
        align_items,
        justify_content
    );
    same!(FI, flex_basis, flex_grow, flex_shrink, align_self);
}
