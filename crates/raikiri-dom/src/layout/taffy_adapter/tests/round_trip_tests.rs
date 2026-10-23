//! A value survives a trip to the library's type and back bit for bit.

use super::*;

/// Run `check` once for each rotation of the extreme floats, so that every
/// field of the value it builds takes each extreme in turn.
fn for_each_rotation(mut check: impl FnMut(&mut Floats)) {
    for start in 0..extreme_floats().len() {
        check(&mut Floats(start));
    }
}

#[test]
fn geometry_keeps_every_bit() {
    for_each_rotation(|floats| {
        assert_round_trip(size(floats));
        assert_round_trip(point(floats));
        assert_round_trip(edges(floats));
        assert_round_trip(option_size(floats));
        assert_round_trip(Line {
            start: floats.next(),
            end: floats.next(),
        });
        assert_round_trip(box_layout(floats));
    });
}

#[test]
fn every_value_of_the_range_survives_in_every_field() {
    for value in extreme_floats() {
        let edges = Edges {
            left: value,
            right: value,
            top: value,
            bottom: value,
        };
        assert_round_trip(edges);
        assert_round_trip(Size {
            width: Some(value),
            height: Some(value),
        });
        assert_round_trip(AvailableSpace::Definite(value));
        assert_round_trip(Baselines {
            first: Some(value),
            last: Some(value),
        });
    }
}

#[test]
fn a_box_layout_keeps_its_order_and_every_field() {
    let layout = BoxLayout {
        order: u32::MAX,
        ..box_layout(&mut Floats(0))
    };
    let back = round_trip(layout);
    assert_eq!(back.order, u32::MAX);
    assert_eq!(bits_of(&back), bits_of(&layout));
    assert_round_trip(BoxLayout::new());
}

#[test]
fn a_node_id_is_the_index_it_was_made_from() {
    for index in [0, 1, 41, u32::MAX as usize, usize::MAX >> 1] {
        let id = NodeId::from(index);
        let taffy_id = id.into_taffy();
        assert_eq!(usize::from(taffy_id), index);
        assert_eq!(NodeId::from_taffy(taffy_id), id);
        assert_eq!(usize::from(round_trip(id)), index);
    }
}

#[test]
fn available_space_keeps_its_variant() {
    for space in [
        AvailableSpace::Definite(f32::NAN),
        AvailableSpace::Definite(-0.0),
        AvailableSpace::MinContent,
        AvailableSpace::MaxContent,
    ] {
        assert_round_trip(space);
    }
}

#[test]
fn a_layout_input_keeps_every_field() {
    for_each_rotation(|floats| assert_round_trip(layout_input(floats)));
    assert_round_trip(LayoutInput::HIDDEN);
}

#[test]
fn static_positions_and_candidates_keep_every_field() {
    for_each_rotation(|floats| {
        assert_round_trip(static_position(floats, AxisStaticEdge::End));
        assert_round_trip(candidate(floats, 11, OofPosition::Absolute));
        assert_round_trip(candidate(floats, 12, OofPosition::Fixed));
        assert_round_trip(OofPositioningArea {
            size: size(floats),
            offset: point(floats),
        });
    });
}

#[test]
fn an_alignment_keeps_its_keyword_safety_and_fallback() {
    for keyword in [
        AxisStaticEdge::Start,
        AxisStaticEdge::Center,
        AxisStaticEdge::End,
    ] {
        for safety in [
            StaticAlignSafety::Default,
            StaticAlignSafety::Unsafe,
            StaticAlignSafety::Safe,
        ] {
            for fallback in [
                AxisStaticEdge::Start,
                AxisStaticEdge::Center,
                AxisStaticEdge::End,
            ] {
                let align = AxisStaticAlign {
                    keyword,
                    safety,
                    fallback,
                };
                assert_eq!(round_trip(align), align);
            }
        }
    }
}

#[test]
fn a_layout_output_keeps_every_field() {
    for_each_rotation(|floats| assert_round_trip(layout_output(floats)));
    assert_round_trip(LayoutOutput::HIDDEN);
    assert_round_trip(LayoutOutput::from_outer_size(Size {
        width: -0.0,
        height: f32::NAN,
    }));
}

#[test]
fn margin_sets_move_across_unchanged() {
    for margin in extreme_floats() {
        let set = CollapsibleMarginSet::from_margin(margin)
            .collapse_with_margin(-2.0)
            .collapse_with_set(CollapsibleMarginSet::from_margin(3.0));
        let taffy_set = set.into_taffy();
        assert_eq!(
            format!("{taffy_set:?}"),
            format!("{:?}", round_trip(set).into_taffy())
        );
        assert_eq!(
            set.resolve().to_bits(),
            taffy_set.resolve().to_bits(),
            "the wrapped set resolves as the library's does"
        );
    }
}

#[test]
fn an_output_that_started_in_the_library_comes_back_as_it_was() {
    // Built by the library's constructors, as its algorithms build them.
    let mut output = taffy::LayoutOutput::from_sizes_and_baselines(
        taffy::Size {
            width: f32::from_bits(0x7fc0_1234),
            height: -0.0,
        },
        taffy::Rect {
            left: 1e7,
            right: -1e7,
            top: -0.0,
            bottom: f32::NEG_INFINITY,
        },
        taffy::Baselines {
            first: Some(f32::NEG_INFINITY),
            last: None,
        },
    );
    output.top_margin = taffy::CollapsibleMarginSet::from_margin(-9.0);
    output.bottom_margin = taffy::CollapsibleMarginSet::from_margin(4.0);
    output.margins_can_collapse_through = true;
    output.oof_positioning_area = Some(taffy::OofPositioningArea {
        size: taffy::Size {
            width: 3.0,
            height: f32::INFINITY,
        },
        offset: taffy::Point { x: -0.0, y: 0.0 },
    });
    let before = format!("{output:?}");
    let back = LayoutOutput::from_taffy(output).into_taffy();
    assert_eq!(format!("{back:?}"), before);
}

#[test]
fn box_layout_helpers_answer_as_the_librarys_do() {
    for start in 0..extreme_floats().len() {
        let layout = box_layout(&mut Floats(start));
        let theirs = layout.into_taffy();
        let same = |own: f32, theirs: f32, what: &str| {
            assert_eq!(own.to_bits(), theirs.to_bits(), "{what} of {layout:?}");
        };
        same(
            layout.content_box_width(),
            theirs.content_box_width(),
            "content box width",
        );
        same(
            layout.content_box_height(),
            theirs.content_box_height(),
            "content box height",
        );
        same(
            layout.content_box_x(),
            theirs.content_box_x(),
            "content box x",
        );
        same(
            layout.content_box_y(),
            theirs.content_box_y(),
            "content box y",
        );
        same(layout.scroll_width(), theirs.scroll_width(), "scroll width");
        same(
            layout.scroll_height(),
            theirs.scroll_height(),
            "scroll height",
        );
        assert_eq!(
            bits_of(&layout.content_box_size()),
            bits_of(&Size::<f32>::from_taffy(theirs.content_box_size()))
        );
    }
}
