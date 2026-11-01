use super::*;
use raikiri_style::property::{
    AlignSelfValue, ContentAlignmentValue, FloatValue, SelfAlignmentValue,
};

#[test]
fn core_claims_and_block_defaults_stay_distinct() {
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
    assert!(view.is_containing_block().fixed);
    let block = view.with_context(ViewContext {
        claims_policy: ClaimsPolicy::Positioning,
        ..ViewContext::default()
    });
    assert!(!block.is_containing_block().absolute);
    assert!(!block.is_containing_block().fixed);
    assert_eq!(block.bridge_align_self(), Some(values::AlignSelf::CENTER));
    assert_eq!(block.bridge_justify_self(), Some(values::AlignSelf::END));
    assert_eq!(
        block.bridge_align_content(),
        values::AlignContent::SPACE_BETWEEN
    );
}

#[test]
fn a_last_baseline_static_position_aligns_to_the_end_not_safe_self_end() {
    use raikiri_style::property::PositionValue;
    let alignments = |position, value| {
        let mut cv = ComputedValues::initial();
        cv.position = position;
        cv.align_self = AlignSelfValue::Value(value);
        cv.justify_self = AlignSelfValue::Value(value);
        let prepared = PreparedStyle::prepare(&cv, StyleMetadata::default());
        let view = LayoutStyleView::new(&prepared, ViewContext::default(), &[], &[]);
        [
            view.bridge_align_self(),
            view.bridge_justify_self(),
            view.static_position_align_self(),
            view.static_position_justify_self(),
        ]
    };
    let last = SelfAlignmentValue::LastBaseline;
    for position in [PositionValue::Absolute, PositionValue::Fixed] {
        let [align, justify, static_align, static_justify] = alignments(position, last);
        assert_eq!(align, Some(values::AlignSelf::SAFE_SELF_END));
        assert_eq!(justify, Some(values::AlignSelf::SAFE_SELF_END));
        assert_eq!(static_align, Some(values::AlignSelf::END));
        assert_eq!(static_justify, Some(values::AlignSelf::END));
    }
    // A box in flow aligns to the baselines it shares with its siblings, and
    // every other alignment reads the same in both places.
    let in_flow = alignments(PositionValue::Static, last);
    assert_eq!(in_flow, [Some(values::AlignSelf::BASELINE); 4]);
    for value in [
        SelfAlignmentValue::Center,
        SelfAlignmentValue::Baseline,
        SelfAlignmentValue::Safe(raikiri_style::property::SelfPosition::End),
    ] {
        let [align, justify, static_align, static_justify] =
            alignments(PositionValue::Absolute, value);
        assert_eq!([static_align, static_justify], [align, justify]);
    }
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
        assert_eq!(view.is_oof_table(), oof);
    }
}

#[test]
fn block_float_and_clear_keep_authored_and_override_values() {
    use raikiri_style::property::ClearValue;
    for (float, expected) in [
        (FloatValue::None, values::Float::None),
        (FloatValue::Left, values::Float::Left),
        (FloatValue::Right, values::Float::Right),
    ] {
        let mut cv = ComputedValues::initial();
        cv.float = float;
        cv.clear = ClearValue::Left;
        let prepared = PreparedStyle::prepare(&cv, StyleMetadata::default());
        let pass = [OwnedPropertyOverride::clear(ClearValue::Both)];
        let view =
            LayoutStyleView::with_owned_overrides(&prepared, ViewContext::default(), &pass, &[]);
        assert_eq!(view.float(), expected);
        assert_eq!(view.clear(), values::Clear::Both);
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
    assert_eq!(view.position(), values::Position::Sticky);
    cv.cssom_writing_mode = WritingMode::VerticalRl;
    cv.min_height = L::Auto;
    cv.min_block_size = Some(L::Px(64.0));
    for (flags, expected) in [
        (
            StyleFlags::BLOCK,
            values::LengthPercentageAuto::length(64.0),
        ),
        (
            StyleFlags::BLOCK.union(StyleFlags::SOURCE_VERTICAL_DIRECTION),
            values::LengthPercentageAuto::auto(),
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

#[test]
fn self_baseline_set_reads_the_own_value_and_falls_back_on_the_container() {
    use crate::values::BaselineSet;
    let auto = AlignSelfValue::Auto;
    let normal = SelfAlignmentValue::Normal;
    let values = |justify_self, align_self, justify_items, align_items| {
        let mut cv = ComputedValues::initial();
        cv.justify_self = justify_self;
        cv.align_self = align_self;
        cv.justify_items = justify_items;
        cv.align_items = align_items;
        cv
    };
    let view = |cv: &ComputedValues, f: &dyn Fn(LayoutStyleView<'_>)| {
        let prepared = PreparedStyle::prepare(cv, StyleMetadata::default());
        f(LayoutStyleView::new(
            &prepared,
            ViewContext::default(),
            &[],
            &[],
        ));
    };
    let container = values(
        auto,
        auto,
        SelfAlignmentValue::Baseline,
        SelfAlignmentValue::LastBaseline,
    );
    view(&container, &|container| {
        // Without a value of its own an item takes the one of the container.
        view(&values(auto, auto, normal, normal), &|item| {
            assert_eq!(
                item.self_baseline_set(&container, true),
                Some(BaselineSet::First)
            );
            assert_eq!(
                item.self_baseline_set(&container, false),
                Some(BaselineSet::Last)
            );
        });
        // A value of the item wins over the container, `normal` included.
        let own = values(
            AlignSelfValue::Value(SelfAlignmentValue::LastBaseline),
            AlignSelfValue::Value(normal),
            normal,
            normal,
        );
        view(&own, &|item| {
            assert_eq!(
                item.self_baseline_set(&container, true),
                Some(BaselineSet::Last)
            );
            assert_eq!(item.self_baseline_set(&container, false), None);
        });
    });
}

#[test]
fn measured_table_stretch_height_does_not_stretch_a_second_time() {
    for height in [L::Stretch, L::Px(40.0)] {
        for swapped in [false, true] {
            let mut cv = ComputedValues::initial();
            cv.width = L::Px(20.0);
            cv.height = height;
            cv.margin.top = L::Auto;
            cv.margin.bottom = L::Auto;
            cv.margin.left = L::Auto;
            cv.margin.right = L::Auto;
            cv.top = L::Px(-10.0);
            cv.bottom = L::Px(-20.0);
            cv.align_self = AlignSelfValue::Value(if swapped {
                SelfAlignmentValue::Center
            } else {
                SelfAlignmentValue::Stretch
            });
            cv.justify_self = AlignSelfValue::Value(if swapped {
                SelfAlignmentValue::Stretch
            } else {
                SelfAlignmentValue::Center
            });
            let prepared = PreparedStyle::prepare(&cv, StyleMetadata::default());
            let plain = LayoutStyleView::new(&prepared, ViewContext::default(), &[], &[]);
            let plain = if swapped {
                plain.with_swapped_oof_alignment()
            } else {
                plain
            };
            let measured = plain.with_hidden_table_height();
            assert_eq!(plain.oof_align_self(), Some(values::AlignSelf::STRETCH));
            let expected = if height == L::Stretch {
                values::AlignSelf::START
            } else {
                values::AlignSelf::STRETCH
            };
            assert_eq!(measured.oof_align_self(), Some(expected));
            assert_eq!(measured.oof_justify_self(), Some(values::AlignSelf::CENTER));
            assert_eq!(measured.bridge_align_self(), plain.bridge_align_self());
            assert_eq!(measured.bridge_justify_self(), plain.bridge_justify_self());
            assert_eq!(measured.inset(), plain.inset());
            if height == L::Stretch {
                assert_eq!(measured.margin().top, LengthPercentageAuto::length(0.0));
                assert_eq!(measured.margin().bottom, LengthPercentageAuto::length(0.0));
                assert_eq!(measured.margin().left, plain.margin().left);
                assert_eq!(measured.margin().right, plain.margin().right);
            } else {
                assert_eq!(measured.margin(), plain.margin());
            }
            assert_eq!(measured.size().width, plain.size().width);
        }
    }
}

#[test]
fn measured_stretch_height_preserves_non_stretch_positioning() {
    let mut cv = ComputedValues::initial();
    cv.height = L::Stretch;
    cv.align_self = AlignSelfValue::Value(SelfAlignmentValue::Safe(
        raikiri_style::property::SelfPosition::Center,
    ));
    cv.margin.top = L::Px(5.0);
    cv.margin.bottom = L::Px(7.0);
    let prepared = PreparedStyle::prepare(&cv, StyleMetadata::default());
    let plain = LayoutStyleView::new(&prepared, ViewContext::default(), &[], &[]);
    let measured = plain.with_hidden_table_height();
    assert_eq!(measured.oof_align_self(), plain.oof_align_self());
    assert_eq!(measured.margin(), plain.margin());
}

#[test]
fn a_measured_stretch_table_cannot_infer_its_height_from_the_width() {
    let mut cv = ComputedValues::initial();
    cv.height = L::Stretch;
    cv.width = L::Px(20.0);
    cv.aspect_ratio.auto = false;
    cv.aspect_ratio.ratio = Some((1.0, 1.0));
    let prepared = PreparedStyle::prepare(&cv, StyleMetadata::default());
    let plain = LayoutStyleView::new(&prepared, ViewContext::default(), &[], &[]);
    assert_eq!(plain.aspect_ratio(), Some(1.0));
    let measured = plain.with_hidden_table_height();
    assert_eq!(measured.aspect_ratio(), None);
    assert_eq!(measured.stated_aspect_ratio(), Some(1.0));
    assert_eq!(measured.size().width, plain.size().width);
}
