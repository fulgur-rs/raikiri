use super::*;
use raikiri_style::property::CalcLengthPercentage;

fn expression(px: f32) -> CalcLengthPercentage {
    CalcLengthPercentage { px, percent: -0.0 }
}

fn scalar_fixture() -> ComputedValues {
    let mut c = ComputedValues::initial();
    c.width = L::Calc(expression(1.0));
    c.height = L::Calc(expression(2.0));
    c.min_width = L::Calc(expression(3.0));
    c.min_height = L::Calc(expression(4.0));
    c.max_width = L::Calc(expression(5.0));
    c.max_height = L::Calc(expression(6.0));
    c.margin.top = L::Calc(expression(7.0));
    c.margin.right = L::Calc(expression(8.0));
    c.margin.bottom = L::Calc(expression(9.0));
    c.margin.left = L::Calc(expression(10.0));
    c.padding.top = P::Calc(expression(11.0));
    c.padding.right = P::Calc(expression(12.0));
    c.padding.bottom = P::Calc(expression(13.0));
    c.padding.left = P::Calc(expression(14.0));
    c.top = L::Calc(expression(15.0));
    c.right = L::Calc(expression(16.0));
    c.bottom = L::Calc(expression(17.0));
    c.left = L::Calc(expression(18.0));
    c.flex_basis = F::Calc(expression(19.0));
    c.column_gap = N::Calc(expression(20.0));
    c.row_gap = N::Calc(expression(21.0));
    c.vertical_logical_size = Some(raikiri_style::VerticalLogicalSize {
        width: L::Calc(expression(22.0)),
        height: L::Calc(expression(23.0)),
    });
    c.min_block_size = Some(L::FitContentOf(P::Calc(expression(24.0))));
    c
}

fn scalar_rows(c: &ComputedValues) -> Vec<(LengthProperty, u32, u32, bool)> {
    let mut rows = Vec::new();
    PreparedStyle::visit_values_calcs(c, |slot, e, range| {
        if let CalcSlot::Property(p) = slot {
            let CalcExpression::Affine(e) = e else {
                panic!("affine fixture")
            };
            rows.push((p, e.px.to_bits(), e.percent.to_bits(), range));
        }
        Ok(())
    })
    .unwrap();
    rows
}

#[test]
fn scalar_visitor_preserves_each_field_range_bits_and_order() {
    use LengthProperty::*;
    let rows = scalar_rows(&scalar_fixture());
    let expected = [
        (Width, true),
        (Height, true),
        (MinWidth, true),
        (MinHeight, true),
        (MaxWidth, true),
        (MaxHeight, true),
        (MarginTop, false),
        (MarginRight, false),
        (MarginBottom, false),
        (MarginLeft, false),
        (PaddingTop, true),
        (PaddingRight, true),
        (PaddingBottom, true),
        (PaddingLeft, true),
        (Top, false),
        (Right, false),
        (Bottom, false),
        (Left, false),
        (FlexBasis, true),
        (ColumnGap, true),
        (RowGap, true),
        (LogicalWidth, true),
        (LogicalHeight, true),
        (MinBlock, true),
    ];
    assert_eq!(rows.len(), expected.len());
    for (i, (&(property, range), actual)) in expected.iter().zip(&rows).enumerate() {
        assert_eq!(
            *actual,
            (
                property,
                ((i + 1) as f32).to_bits(),
                (-0.0_f32).to_bits(),
                range
            )
        );
    }
}

#[test]
fn scalar_visitor_omits_absent_logical_aliases_and_stops_on_first_error() {
    let mut c = scalar_fixture();
    c.vertical_logical_size = None;
    c.min_block_size = None;
    c.column_gap = N::Normal;
    c.row_gap = N::Px(3.0);
    let rows = scalar_rows(&c);
    assert_eq!(rows.len(), 19);
    assert_eq!(rows.last().unwrap().0, LengthProperty::FlexBasis);
    for (stop, row) in rows.iter().enumerate() {
        let mut visited = Vec::new();
        let result = PreparedStyle::visit_values_calcs(&c, |slot, _, _| {
            visited.push(slot);
            if visited.len() == stop + 1 {
                Err(PrepareError::Missing(slot))
            } else {
                Ok(())
            }
        });
        let expected = CalcSlot::Property(row.0);
        assert_eq!(result, Err(PrepareError::Missing(expected)));
        assert_eq!(visited.len(), stop + 1);
        assert_eq!(visited.last(), Some(&expected));
    }
}

#[test]
fn scalar_scan_matches_dynamic_queries_for_keywords_and_optional_sources() {
    for value in [
        L::Auto,
        L::MinContent,
        L::MaxContent,
        L::FitContent,
        L::Px(7.0),
        L::Percent(33.0),
        L::FitContentOf(P::Px(9.0)),
        L::Calc(expression(-0.0)),
        L::FitContentOf(P::Calc(expression(f32::from_bits(0x7fc0_0001)))),
    ] {
        for logical in [false, true] {
            let mut c = scalar_fixture();
            c.width = value.clone();
            c.min_block_size = Some(value.clone());
            if logical {
                c.vertical_logical_size.as_mut().unwrap().height = value.clone();
            } else {
                c.vertical_logical_size = None;
            }
            let mut expected = Vec::new();
            for &property in LengthProperty::ALL {
                if !logical
                    && matches!(
                        property,
                        LengthProperty::LogicalWidth | LengthProperty::LogicalHeight
                    )
                {
                    continue;
                }
                if let L::Calc(e) | L::FitContentOf(P::Calc(e)) =
                    PreparedStyle::scalar_value(&c, property).as_ref()
                {
                    expected.push((
                        property,
                        e.px.to_bits(),
                        e.percent.to_bits(),
                        property.non_negative(),
                    ));
                }
            }
            assert_eq!(scalar_rows(&c), expected);
        }
    }
}
