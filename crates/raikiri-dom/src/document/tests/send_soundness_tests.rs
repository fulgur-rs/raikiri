use super::*;
use taffy::Style;

fn external_calc_ptr() -> *const () {
    static TOKEN: usize = 0;
    (&TOKEN as *const usize).cast::<()>()
}

fn calc_track() -> taffy::TrackSizingFunction {
    taffy::TrackSizingFunction {
        min: taffy::MinTrackSizingFunction::calc(external_calc_ptr()),
        max: taffy::MaxTrackSizingFunction::calc(external_calc_ptr()),
    }
}

fn style_has_no_calc(style: &Style) -> bool {
    let scalars = [
        style.size.width.into_raw().is_calc(),
        style.size.height.into_raw().is_calc(),
        style.min_size.width.into_raw().is_calc(),
        style.min_size.height.into_raw().is_calc(),
        style.max_size.width.into_raw().is_calc(),
        style.max_size.height.into_raw().is_calc(),
        style.inset.left.into_raw().is_calc(),
        style.inset.right.into_raw().is_calc(),
        style.inset.top.into_raw().is_calc(),
        style.inset.bottom.into_raw().is_calc(),
        style.margin.left.into_raw().is_calc(),
        style.margin.right.into_raw().is_calc(),
        style.margin.top.into_raw().is_calc(),
        style.margin.bottom.into_raw().is_calc(),
        style.padding.left.into_raw().is_calc(),
        style.padding.right.into_raw().is_calc(),
        style.padding.top.into_raw().is_calc(),
        style.padding.bottom.into_raw().is_calc(),
        style.border.left.into_raw().is_calc(),
        style.border.right.into_raw().is_calc(),
        style.border.top.into_raw().is_calc(),
        style.border.bottom.into_raw().is_calc(),
        style.gap.width.into_raw().is_calc(),
        style.gap.height.into_raw().is_calc(),
        style.flex_basis.into_raw().is_calc(),
    ];
    if scalars.into_iter().any(|is_calc| is_calc) {
        return false;
    }
    let mut tracks: Vec<taffy::TrackSizingFunction> = Vec::new();
    for component in style
        .grid_template_rows
        .iter()
        .chain(style.grid_template_columns.iter())
    {
        match component {
            taffy::GridTemplateComponent::Single(track) => tracks.push(*track),
            taffy::GridTemplateComponent::Repeat(repetition) => {
                tracks.extend(repetition.tracks.iter().copied());
            }
        }
    }
    tracks.extend(style.grid_auto_rows.iter().copied());
    tracks.extend(style.grid_auto_columns.iter().copied());
    !tracks
        .into_iter()
        .any(|track| track.min.into_raw().is_calc() || track.max.into_raw().is_calc())
}

#[test]
fn external_calc_in_size_is_replaced_with_auto() {
    let mut doc = Document::new();
    let style = Style {
        size: taffy::Size {
            width: taffy::Dimension::calc(external_calc_ptr()),
            height: taffy::Dimension::calc(external_calc_ptr()),
        },
        ..Style::default()
    };
    let id = doc.append_element(Some(0), "div", style, None::<&str>);
    let stored = doc.layout_style(id).expect("appended node has a style");
    assert!(!stored.size.width.into_raw().is_calc());
    assert!(!stored.size.height.into_raw().is_calc());
    assert_eq!(stored.size.width, taffy::Dimension::auto());
    assert_eq!(stored.size.height, taffy::Dimension::auto());
    assert!(style_has_no_calc(stored));
}

#[test]
fn external_calc_in_box_fields_is_sanitized() {
    let mut doc = Document::new();
    let style = Style {
        min_size: taffy::Size {
            width: taffy::LengthPercentageAuto::calc(external_calc_ptr()),
            height: taffy::LengthPercentageAuto::auto(),
        },
        max_size: taffy::Size {
            width: taffy::LengthPercentageAuto::auto(),
            height: taffy::LengthPercentageAuto::calc(external_calc_ptr()),
        },
        inset: taffy::Rect {
            left: taffy::LengthPercentageAuto::calc(external_calc_ptr()),
            right: taffy::LengthPercentageAuto::auto(),
            top: taffy::LengthPercentageAuto::auto(),
            bottom: taffy::LengthPercentageAuto::auto(),
        },
        margin: taffy::Rect {
            left: taffy::LengthPercentageAuto::auto(),
            right: taffy::LengthPercentageAuto::auto(),
            top: taffy::LengthPercentageAuto::calc(external_calc_ptr()),
            bottom: taffy::LengthPercentageAuto::auto(),
        },
        padding: taffy::Rect {
            left: taffy::LengthPercentage::calc(external_calc_ptr()),
            right: taffy::LengthPercentage::length(0.0),
            top: taffy::LengthPercentage::length(0.0),
            bottom: taffy::LengthPercentage::length(0.0),
        },
        border: taffy::Rect {
            left: taffy::LengthPercentage::length(0.0),
            right: taffy::LengthPercentage::length(0.0),
            top: taffy::LengthPercentage::length(0.0),
            bottom: taffy::LengthPercentage::calc(external_calc_ptr()),
        },
        gap: taffy::Size {
            width: taffy::LengthPercentage::calc(external_calc_ptr()),
            height: taffy::LengthPercentage::length(0.0),
        },
        flex_basis: taffy::Dimension::calc(external_calc_ptr()),
        ..Style::default()
    };
    let id = doc.append_element(Some(0), "div", style, None::<&str>);
    let stored = doc.layout_style(id).expect("appended node has a style");
    assert_eq!(stored.min_size.width, taffy::LengthPercentageAuto::auto());
    assert_eq!(stored.max_size.height, taffy::LengthPercentageAuto::auto());
    assert_eq!(stored.inset.left, taffy::LengthPercentageAuto::auto());
    assert_eq!(stored.margin.top, taffy::LengthPercentageAuto::auto());
    assert_eq!(stored.padding.left, taffy::LengthPercentage::length(0.0));
    assert_eq!(stored.border.bottom, taffy::LengthPercentage::length(0.0));
    assert_eq!(stored.gap.width, taffy::LengthPercentage::length(0.0));
    assert_eq!(stored.flex_basis, taffy::Dimension::auto());
    assert!(style_has_no_calc(stored));
}

#[test]
fn external_calc_in_grid_tracks_is_sanitized() {
    let mut doc = Document::new();
    let style = Style {
        grid_template_columns: vec![taffy::GridTemplateComponent::Single(calc_track())],
        grid_template_rows: vec![taffy::GridTemplateComponent::Repeat(
            taffy::GridTemplateRepetition {
                count: taffy::RepetitionCount::Count(2),
                tracks: vec![calc_track()],
                line_names: Vec::new(),
            },
        )],
        grid_auto_rows: vec![calc_track()],
        grid_auto_columns: vec![calc_track()],
        ..Style::default()
    };
    let id = doc.append_element(Some(0), "div", style, None::<&str>);
    let stored = doc.layout_style(id).expect("appended node has a style");
    assert!(style_has_no_calc(stored));
    for component in stored
        .grid_template_rows
        .iter()
        .chain(stored.grid_template_columns.iter())
    {
        match component {
            taffy::GridTemplateComponent::Single(track) => {
                assert_eq!(track.min, taffy::MinTrackSizingFunction::auto());
                assert_eq!(track.max, taffy::MaxTrackSizingFunction::auto());
            }
            taffy::GridTemplateComponent::Repeat(repetition) => {
                assert_eq!(repetition.count, taffy::RepetitionCount::Count(2));
                for track in repetition.tracks.iter() {
                    assert_eq!(track.min, taffy::MinTrackSizingFunction::auto());
                    assert_eq!(track.max, taffy::MaxTrackSizingFunction::auto());
                }
            }
        }
    }
}

#[test]
fn normal_styles_pass_through_unchanged() {
    let mut doc = Document::new();
    let style = Style {
        size: taffy::Size {
            width: taffy::Dimension::length(100.0),
            height: taffy::Dimension::percent(0.5),
        },
        margin: taffy::Rect {
            left: taffy::LengthPercentageAuto::length(4.0),
            right: taffy::LengthPercentageAuto::auto(),
            top: taffy::LengthPercentageAuto::percent(0.1),
            bottom: taffy::LengthPercentageAuto::length(2.0),
        },
        padding: taffy::Rect {
            left: taffy::LengthPercentage::length(3.0),
            right: taffy::LengthPercentage::length(3.0),
            top: taffy::LengthPercentage::percent(0.2),
            bottom: taffy::LengthPercentage::length(1.0),
        },
        flex_basis: taffy::Dimension::length(10.0),
        ..Style::default()
    };
    let expected = style.clone();
    let id = doc.append_element(Some(0), "div", style, None::<&str>);
    let stored = doc.layout_style(id).expect("appended node has a style");
    assert_eq!(*stored, expected);
    assert!(style_has_no_calc(stored));
}

#[test]
fn replace_children_from_does_not_adopt_foreign_calc() {
    let mut source = Document::new();
    let src_parent = source.append_element(Some(0), "div", Style::default(), None::<&str>);
    let src_child = source.append_element(Some(src_parent), "span", Style::default(), None::<&str>);
    source.nodes[src_child].style.size.width = taffy::Dimension::calc(external_calc_ptr());
    let mut target = Document::new();
    let target_parent = target.append_element(Some(0), "main", Style::default(), None::<&str>);
    target.replace_children_from(target_parent, &source, src_parent);
    let copied = target.nodes[target_parent].children[0];
    let stored = target
        .layout_style(copied)
        .expect("copied node has a style");
    assert!(style_has_no_calc(stored));
    assert_eq!(stored.size.width, taffy::Dimension::auto());
}

#[test]
fn sanitized_document_can_move_across_threads() {
    fn assert_send<T: Send>() {}
    assert_send::<Document>();
    let mut doc = Document::new();
    let style = Style {
        size: taffy::Size {
            width: taffy::Dimension::calc(external_calc_ptr()),
            height: taffy::Dimension::length(20.0),
        },
        ..Style::default()
    };
    doc.append_element(Some(0), "div", style, None::<&str>);
    std::thread::scope(|scope| {
        scope
            .spawn(move || {
                let _ = doc.node_count();
            })
            .join()
            .expect("sanitized document moves across threads");
    });
}
