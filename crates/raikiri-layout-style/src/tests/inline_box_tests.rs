use super::*;
use raikiri_style::ComputedLengthPercentageOrCalc as P;
use raikiri_style::property::{BorderStyle, Length, WritingMode};

#[test]
fn inline_box_normalization_uses_source_axes_before_query_projection() {
    let mut cv = ComputedValues::initial();
    cv.width = L::Px(30.0);
    cv.height = L::Px(20.0);
    cv.min_width = L::Px(50.0);
    cv.min_height = L::Px(50.0);
    cv.max_width = L::Px(5.0);
    cv.max_height = L::Px(5.0);
    cv.margin.top = L::Px(1.0);
    cv.margin.right = L::Px(2.0);
    cv.margin.bottom = L::Px(3.0);
    cv.margin.left = L::Px(4.0);
    cv.padding.top = P::Px(1.0);
    cv.padding.right = P::Px(2.0);
    cv.padding.bottom = P::Px(3.0);
    cv.padding.left = P::Px(4.0);
    let mut border = raikiri_style::SpecifiedValues::initial().border.left;
    border.width = Length::Px(5.0);
    border.style = BorderStyle::Solid;
    let border = raikiri_style::resolve_border(
        border,
        raikiri_style::ComputedLength(16.0),
        None,
        &raikiri_style::ResolveContext::initial(),
    );
    cv.border.top = border;
    cv.border.right = border;
    cv.border.bottom = border;
    cv.border.left = border;
    for (source, query, edges, borders) in [
        (
            WritingMode::VerticalRl,
            WritingMode::HorizontalTb,
            [1.0, 0.0, 3.0, 0.0],
            [5.0, 0.0, 5.0, 0.0],
        ),
        (
            WritingMode::VerticalLr,
            WritingMode::VerticalRl,
            [0.0, 3.0, 0.0, 1.0],
            [0.0, 5.0, 0.0, 5.0],
        ),
        (
            WritingMode::HorizontalTb,
            WritingMode::VerticalRl,
            [2.0, 0.0, 4.0, 0.0],
            [5.0, 0.0, 5.0, 0.0],
        ),
    ] {
        let prepared = PreparedStyle::new(
            &cv,
            StyleMetadata {
                flags: StyleFlags::INLINE_BOX,
                used_writing_mode: source,
                ..StyleMetadata::default()
            },
            &[],
        )
        .unwrap();
        let view = LayoutStyleView::new(
            &prepared,
            ViewContext {
                layout_wm: query,
                ..ViewContext::default()
            },
            &[],
            &[],
        );
        assert_eq!(
            view.size(),
            Size {
                width: Dimension::auto(),
                height: Dimension::auto()
            }
        );
        for sizes in [view.min_size(), view.max_size()] {
            assert_eq!(
                sizes,
                Size {
                    width: LengthPercentageAuto::auto(),
                    height: LengthPercentageAuto::auto()
                }
            );
        }
        let [top, right, bottom, left] = edges.map(LengthPercentageAuto::length);
        assert_eq!(
            view.margin(),
            Edges {
                top,
                right,
                bottom,
                left
            }
        );
        let [top, right, bottom, left] = edges.map(LengthPercentage::length);
        assert_eq!(
            view.padding(),
            Edges {
                top,
                right,
                bottom,
                left
            }
        );
        let [top, right, bottom, left] = borders.map(LengthPercentage::length);
        assert_eq!(
            view.border(),
            Edges {
                top,
                right,
                bottom,
                left
            }
        );
    }
}
