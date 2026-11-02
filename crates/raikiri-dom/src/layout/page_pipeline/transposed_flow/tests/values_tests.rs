use super::*;
use crate::layout::ifc::test_support::block_fixture;
use raikiri_style::ComputedLengthPercentageOrAuto as Length;
use raikiri_style::computed::RunningTemplate;

#[test]
fn transposition_preserves_owned_buffers_and_original_values() {
    for right_to_left in [false, true] {
        for mode in ["horizontal-tb", "vertical-rl", "vertical-lr"] {
            let mut fixture = block_fixture(
                &format!(
                    "writing-mode:{mode};width:30px;height:60px;min-width:5px;min-height:6px;max-width:70px;max-height:80px;margin:1px 2px 3px 4px;top:25%;right:50%;bottom:75%;left:100%"
                ),
                |doc, root| {
                    doc.append_text(root, "x");
                },
            );
            fixture.cascade.computed[fixture.root]
                .running_templates
                .push(RunningTemplate::new("buffer-kept-by-transposition"));
            let original = &fixture.cascade.computed[fixture.root];
            let mut values = original.clone();
            let pointer = values.running_templates.as_ptr();
            let snapshot = original.clone();
            FlowAxes { right_to_left }.transpose_computed(
                &fixture.cascade,
                fixture.root,
                &mut values,
            );
            assert_eq!(values.running_templates.as_ptr(), pointer);
            assert_eq!(values.running_templates.len(), 1);
            assert_eq!(original, &snapshot);
            assert_eq!(
                (values.width, values.height),
                (Length::Px(60.0), Length::Px(30.0))
            );
            assert_eq!(
                (values.min_width, values.min_height),
                (Length::Px(6.0), Length::Px(5.0))
            );
            assert_eq!(
                (values.max_width, values.max_height),
                (Length::Px(80.0), Length::Px(70.0))
            );
            assert!(values.vertical_logical_size.is_none());
            let (margins, insets) = if right_to_left {
                ([2.0, 3.0, 4.0, 1.0], [50.0, 75.0, 100.0, 25.0])
            } else {
                ([4.0, 3.0, 2.0, 1.0], [100.0, 75.0, 50.0, 25.0])
            };
            assert_eq!(
                [
                    values.margin.top,
                    values.margin.right,
                    values.margin.bottom,
                    values.margin.left
                ],
                margins.map(Length::Px)
            );
            assert_eq!(
                [values.top, values.right, values.bottom, values.left],
                insets.map(Length::Percent)
            );
        }
    }
}

#[test]
fn in_place_transposition_reads_the_original_vertical_logical_size() {
    for mode in ["vertical-rl", "vertical-lr"] {
        let fixture = block_fixture(
            &format!("writing-mode:{mode};inline-size:90px;block-size:40px"),
            |doc, root| {
                doc.append_text(root, "x");
            },
        );
        let original = &fixture.cascade.computed[fixture.root];
        let size = original.vertical_logical_size.as_ref().unwrap();
        assert_eq!(
            (size.width.clone(), size.height.clone()),
            (Length::Px(40.0), Length::Px(90.0))
        );
        let mut values = original.clone();
        values.vertical_logical_size = None;
        FlowAxes {
            right_to_left: mode == "vertical-rl",
        }
        .transpose_computed(&fixture.cascade, fixture.root, &mut values);
        assert_eq!(
            (values.width, values.height),
            (Length::Px(90.0), Length::Px(40.0))
        );
        assert!(values.vertical_logical_size.is_none());
        assert!(original.vertical_logical_size.is_some());
    }
}
