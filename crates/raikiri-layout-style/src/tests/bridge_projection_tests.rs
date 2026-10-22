use super::*;
use raikiri_style::property::{Contain, Direction, OverflowValue, Sides, WritingMode};

#[test]
fn bridge_projection_changes_only_geometry() {
    let mut cv = ComputedValues::initial();
    cv.width = L::Px(10.0);
    cv.height = L::Px(20.0);
    cv.min_width = L::Px(5.0);
    cv.max_width = L::Px(15.0);
    cv.max_height = L::Px(16.0);
    cv.top = L::Px(1.0);
    cv.right = L::Px(2.0);
    cv.bottom = L::Px(3.0);
    cv.left = L::Px(4.0);
    cv.margin = Sides {
        top: L::Px(1.0),
        right: L::Px(2.0),
        bottom: L::Px(3.0),
        left: L::Px(4.0),
    };
    cv.overflow.x = OverflowValue::Hidden;
    cv.direction = Direction::Rtl;
    cv.contain = Contain::LAYOUT;
    cv.cssom_writing_mode = WritingMode::VerticalRl;
    cv.min_block_size = Some(L::Px(17.0));
    let prepared = PreparedStyle::prepare(&cv, StyleMetadata::default());
    let pass = [PropertyOverride::aspect_ratio(Some(2.0))];
    let plain = LayoutStyleView::new(&prepared, ViewContext::default(), &pass, &[]);
    for right_to_left in [false, true] {
        let view = plain.with_context(ViewContext {
            bridge_projection: BridgeProjection::BlockTranspose { right_to_left },
            ..ViewContext::default()
        });
        assert_eq!(
            view.size(),
            taffy::Size {
                width: taffy::Dimension::length(20.0),
                height: taffy::Dimension::length(10.0)
            }
        );
        assert_eq!(view.aspect_ratio(), Some(0.5));
        assert_eq!(
            view.overflow(),
            taffy::Point {
                x: plain.overflow().y,
                y: plain.overflow().x
            }
        );
        assert_eq!(
            view.margin(),
            taffy::Rect {
                left: taffy::LengthPercentageAuto::length(1.0),
                right: taffy::LengthPercentageAuto::length(3.0),
                top: taffy::LengthPercentageAuto::length(if right_to_left { 2.0 } else { 4.0 }),
                bottom: taffy::LengthPercentageAuto::length(if right_to_left { 4.0 } else { 2.0 }),
            }
        );
        assert_eq!(view.inset(), view.margin());
        assert_eq!(
            view.min_size().width,
            taffy::LengthPercentageAuto::length(17.0)
        );
        assert_eq!(
            view.min_size().height,
            taffy::LengthPercentageAuto::length(5.0)
        );
        assert_eq!(
            view.max_size(),
            taffy::Size {
                width: taffy::LengthPercentageAuto::length(16.0),
                height: taffy::LengthPercentageAuto::length(15.0)
            }
        );
        assert_eq!(view.direction(), plain.direction());
        assert_eq!(view.contain(), plain.contain());
    }
}

#[test]
fn block_item_basis_resolves_only_output_vertical_edges() {
    let mut cv = ComputedValues::initial();
    cv.padding = Sides::all(raikiri_style::ComputedLengthPercentageOrCalc::Percent(10.0));
    let prepared = PreparedStyle::prepare(&cv, StyleMetadata::default());
    let plain = LayoutStyleView::new(&prepared, ViewContext::default(), &[], &[]);
    for projection in [
        BridgeProjection::Identity,
        BridgeProjection::BlockTranspose {
            right_to_left: true,
        },
    ] {
        let view = plain.with_context(ViewContext {
            bridge_projection: projection,
            block_item_basis: Some(100.0),
            ..ViewContext::default()
        });
        assert_eq!(
            view.padding(),
            taffy::Rect {
                top: taffy::LengthPercentage::length(10.0),
                bottom: taffy::LengthPercentage::length(10.0),
                left: taffy::LengthPercentage::percent(0.1),
                right: taffy::LengthPercentage::percent(0.1)
            }
        );
    }
    assert_eq!(plain.padding().top, taffy::LengthPercentage::percent(0.1));
}

#[test]
fn undo_restores_only_its_property() {
    let value = |p, v| OwnedPropertyOverride::length(p, L::Px(v), true).unwrap();
    let mut entries = vec![value(LengthProperty::Width, 10.0)];
    let undo = value(LengthProperty::Width, 20.0).replace_in(&mut entries);
    value(LengthProperty::Height, 91.0).install(&mut entries);
    undo.restore(&mut entries);
    let cv = ComputedValues::initial();
    let prepared = PreparedStyle::prepare(&cv, StyleMetadata::default());
    let view =
        LayoutStyleView::with_owned_overrides(&prepared, ViewContext::default(), &entries, &[]);
    assert_eq!(
        view.size(),
        taffy::Size {
            width: taffy::Dimension::length(10.0),
            height: taffy::Dimension::length(91.0)
        }
    );
    let undo = value(LengthProperty::MaxWidth, 20.0).replace_in(&mut entries);
    value(LengthProperty::MaxHeight, 91.0).install(&mut entries);
    undo.restore(&mut entries);
    let view =
        LayoutStyleView::with_owned_overrides(&prepared, ViewContext::default(), &entries, &[]);
    assert!(view.max_size().width.is_auto());
    assert_eq!(
        view.max_size().height,
        taffy::LengthPercentageAuto::length(91.0)
    );
}

#[test]
fn block_basis_preserves_existing_resolved_edges_and_calc_owners() {
    use raikiri_style::{ComputedLengthPercentageOrCalc as P, property::CalcLengthPercentage};
    let mut cv = ComputedValues::initial();
    cv.padding.top = P::Calc(CalcLengthPercentage {
        px: 2.0,
        percent: 10.0,
    });
    cv.padding.right = P::Percent(20.0);
    cv.padding.bottom = P::Px(7.0);
    cv.padding.left = P::Percent(30.0);
    let prepared = PreparedStyle::prepare(&cv, StyleMetadata::default());
    let view = LayoutStyleView::new(
        &prepared,
        ViewContext {
            block_item_basis: Some(100.0),
            ..ViewContext::default()
        },
        &[],
        &[],
    );
    assert_eq!(view.padding().top, taffy::LengthPercentage::length(12.0));
    assert_eq!(view.padding().bottom, taffy::LengthPercentage::length(7.0));
    assert_eq!(view.padding().right, taffy::LengthPercentage::percent(0.2));
    let view = view.with_context(ViewContext {
        percent_basis: PercentBasis::Definite(200.0),
        block_item_basis: Some(100.0),
        ..ViewContext::default()
    });
    assert_eq!(view.padding().top, taffy::LengthPercentage::length(22.0));
    assert_eq!(view.padding().right, taffy::LengthPercentage::length(40.0));
    let pass = [
        PropertyOverride::length(LengthProperty::PaddingTop, LengthValue::resolved(1e8)).unwrap(),
    ];
    let view = LayoutStyleView::new(
        &prepared,
        ViewContext {
            block_item_basis: Some(100.0),
            ..ViewContext::default()
        },
        &pass,
        &[],
    );
    assert_eq!(view.padding().top, taffy::LengthPercentage::length(1e8));
}
