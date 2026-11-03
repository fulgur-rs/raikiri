use super::*;
use raikiri_style::property::{AlignSelfValue, DisplayValue, SelfAlignmentValue};

#[test]
fn the_parent_measures_collapsed_width_without_reapplying_bounds_or_ratio() {
    let mut cv = ComputedValues::initial();
    cv.display = DisplayValue::Table;
    cv.width = L::Px(150.0);
    cv.min_width = L::Px(80.0);
    cv.max_width = L::Px(200.0);
    cv.height = L::Px(20.0);
    cv.aspect_ratio.auto = false;
    cv.aspect_ratio.ratio = Some((2.0, 1.0));
    let prepared = PreparedStyle::prepare(&cv, StyleMetadata::default());
    for held in [
        None,
        Some(HeldCrossSize {
            axis: Axis::X,
            min: true,
            max: true,
        }),
    ] {
        let own = LayoutStyleView::new(
            &prepared,
            ViewContext {
                held_cross_size: held,
                ..ViewContext::default()
            },
            &[],
            &[],
        );
        let original = (
            own.size(),
            own.min_size(),
            own.max_size(),
            own.aspect_ratio(),
        );
        let parent = own.with_collapsing_table_width(
            values::AlignItems::NORMAL,
            values::AlignItems::NORMAL,
            false,
        );
        assert!(parent.size().width.is_auto());
        assert!(parent.min_size().width.is_auto());
        assert!(parent.max_size().width.is_auto());
        assert_eq!(parent.size().height, own.size().height);
        assert_eq!(parent.aspect_ratio(), None);
        assert_eq!(parent.stated_aspect_ratio(), Some(2.0));
        assert_eq!(
            (
                own.size(),
                own.min_size(),
                own.max_size(),
                own.aspect_ratio()
            ),
            original,
        );
    }
}

#[test]
fn an_authored_width_keeps_its_alignment_when_the_parent_reads_auto() {
    for transposed in [false, true] {
        for flex in [false, true] {
            for (alignment, expected) in [
                (AlignSelfValue::Auto, values::AlignItems::START),
                (
                    AlignSelfValue::Value(SelfAlignmentValue::Center),
                    values::AlignItems::CENTER,
                ),
                (
                    AlignSelfValue::Value(SelfAlignmentValue::End),
                    values::AlignItems::END,
                ),
            ] {
                let mut cv = ComputedValues::initial();
                cv.display = DisplayValue::Table;
                cv.width = L::Px(150.0);
                cv.align_self = alignment;
                cv.justify_self = alignment;
                let prepared = PreparedStyle::prepare(&cv, StyleMetadata::default());
                let context = ViewContext {
                    bridge_projection: if transposed {
                        BridgeProjection::BlockTranspose {
                            right_to_left: false,
                        }
                    } else {
                        BridgeProjection::Identity
                    },
                    ..ViewContext::default()
                };
                let parent = LayoutStyleView::new(&prepared, context, &[], &[])
                    .with_collapsing_table_width(
                        values::AlignItems::NORMAL,
                        values::AlignItems::NORMAL,
                        flex,
                    );
                let actual = if flex {
                    parent.align_self()
                } else if transposed {
                    parent.bridge_align_self()
                } else {
                    parent.bridge_justify_self()
                };
                assert_eq!(actual, Some(expected));
            }
        }
    }
}
