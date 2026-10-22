use super::*;
use crate::layout::{
    bridge::apply_computed_to_style_in_session, context::LayoutSession,
    style_views::tests::fixture_with_styles,
};
use crate::{LayoutStyle, node::arena::BoxKey};
use raikiri_layout_style::{
    AlignedCalc, LayoutStyleView, PreparedStyle, StyleMetadata, ViewContext,
};
use raikiri_style::{
    ComputedLengthPercentageOrAuto as L, ComputedValues, SmolStr, property::CalcLengthPercentage,
};
use taffy::{CoreStyle, GenericRepetition, GridContainerStyle};

fn context(role: ReadRole) -> ReadContext {
    ReadContext::new(BoxKey::Source(7), role)
}

fn panic_text(error: Box<dyn std::any::Any + Send>) -> String {
    if let Some(value) = error.downcast_ref::<String>() {
        value.clone()
    } else {
        error.downcast_ref::<&str>().unwrap().to_string()
    }
}

#[test]
fn checker_detects_mapping_fault_at_read() {
    let mut cv = ComputedValues::initial();
    cv.width = L::Px(31.0);
    let prepared = PreparedStyle::prepare(&cv, StyleMetadata::default());
    let view = LayoutStyleView::new(&prepared, ViewContext::default(), &[], &[]);
    let old = LayoutStyle {
        size: taffy::Size {
            width: taffy::Dimension::length(99.0),
            height: taffy::Dimension::auto(),
        },
        ..LayoutStyle::default()
    };
    let stats = ComparisonStats::default();
    let checked = CheckedStyle::new(&old, Some(view), context(ReadRole::FlexItem), &stats);
    assert_eq!(stats.getter_reads[ReadRole::FlexItem as usize].get(), 0);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| checked.size()));
    let message = panic_text(result.unwrap_err());
    for needle in [
        "node=Source(7)",
        "CoreStyle::size",
        "width",
        "legacy=",
        "view=",
        "context=",
        "scope=",
    ] {
        assert!(message.contains(needle), "{message}: missing {needle}");
    }
    assert_eq!(stats.getter_reads[ReadRole::FlexItem as usize].get(), 1);
}

#[test]
fn equal_calcs_at_different_addresses_are_accepted() {
    let expression = CalcLengthPercentage {
        px: -10.0,
        percent: 25.0,
    };
    let mut owners = Vec::new();
    let pointer = crate::taffy_impl::push_calc(&mut owners, expression, true);
    let old = LayoutStyle {
        size: taffy::Size {
            width: taffy::Dimension::calc(pointer),
            height: taffy::Dimension::auto(),
        },
        ..LayoutStyle::default()
    };
    let mut cv = ComputedValues::initial();
    cv.width = L::Calc(expression);
    let prepared = PreparedStyle::prepare(&cv, StyleMetadata::default());
    let view = LayoutStyleView::new(&prepared, ViewContext::default(), &[], &[]);
    assert_ne!(pointer, view.size().width.into_raw().calc_value());
    let stats = ComparisonStats::default();
    let checked = CheckedStyle::new(&old, Some(view), context(ReadRole::FlexItem), &stats);
    assert_eq!(checked.size(), old.size);
    assert!(stats.calc_comparisons.get() >= 5);
}

#[test]
fn calc_comparison_uses_bases_and_ranges() {
    for (legacy, candidate, non_negative) in [
        (
            CalcLengthPercentage {
                px: -10.0,
                percent: 0.001,
            },
            CalcLengthPercentage {
                px: -10.0,
                percent: 0.002,
            },
            true,
        ),
        (
            CalcLengthPercentage {
                px: -10.0,
                percent: 25.0,
            },
            CalcLengthPercentage {
                px: -10.0,
                percent: 25.0,
            },
            false,
        ),
    ] {
        let mut owners = Vec::new();
        let pointer = crate::taffy_impl::push_calc(&mut owners, legacy, non_negative);
        if non_negative {
            let candidate_value = AlignedCalc::new(candidate, true);
            for basis in [0.0, 1.0, 37.0, 100.0, 1000.0] {
                assert_eq!(
                    crate::taffy_impl::resolve_calc(pointer, basis),
                    candidate_value.resolve(basis)
                );
            }
        }
        let old = LayoutStyle {
            size: taffy::Size {
                width: taffy::Dimension::calc(pointer),
                height: taffy::Dimension::auto(),
            },
            ..LayoutStyle::default()
        };
        let mut cv = ComputedValues::initial();
        cv.width = L::Calc(candidate);
        let prepared = PreparedStyle::prepare(&cv, StyleMetadata::default());
        let view = LayoutStyleView::new(&prepared, ViewContext::default(), &[], &[]);
        let stats = ComparisonStats::default();
        let checked = CheckedStyle::new(&old, Some(view), context(ReadRole::FlexItem), &stats);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| checked.size()));
        let message = panic_text(result.unwrap_err());
        assert!(message.contains("basis="), "{message}");
    }
}

#[test]
fn grid_consumption_checks_repeat_names_and_tail() {
    let (mut doc, cascade, child, _) = fixture_with_styles(
        "display:grid;grid-template-columns:repeat(2,[inside] 10px [end])",
        "width:1px",
    );
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
    let base = tree.layout_style_for_key(parent).unwrap().clone();
    let view = tree.style_view(parent);
    for fault in ["count", "track", "names", "tail"] {
        let mut old = base.clone();
        if fault == "tail" {
            old.grid_template_columns
                .push(taffy::GridTemplateComponent::Single(
                    taffy::TrackSizingFunction {
                        min: taffy::MinTrackSizingFunction::auto(),
                        max: taffy::MaxTrackSizingFunction::auto(),
                    },
                ));
        } else {
            let taffy::GridTemplateComponent::Repeat(repeat) = &mut old.grid_template_columns[0]
            else {
                panic!("repeat fixture");
            };
            match fault {
                "count" => repeat.count = taffy::RepetitionCount::Count(3),
                "track" => repeat.tracks[0].min = taffy::MinTrackSizingFunction::length(19.0),
                "names" => repeat.line_names[0][0] = SmolStr::new("wrong"),
                _ => unreachable!(),
            }
        }
        let stats = ComparisonStats::default();
        let checked = CheckedStyle::new(&old, Some(view), context(ReadRole::GridContainer), &stats);
        let tracks = checked.grid_template_columns().unwrap();
        assert_eq!(stats.iterator_reads.get(), 0);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for component in tracks {
                match component {
                    taffy::GenericGridTemplateComponent::Single(track) => {
                        std::hint::black_box(track);
                    }
                    taffy::GenericGridTemplateComponent::Repeat(repeat) => {
                        std::hint::black_box(repeat.count());
                        for track in repeat.tracks() {
                            std::hint::black_box(track);
                        }
                        for names in repeat.lines_names() {
                            for name in names {
                                std::hint::black_box(name);
                            }
                        }
                    }
                }
            }
        }));
        assert!(result.is_err(), "missing {fault} detection");
        assert!(stats.iterator_reads.get() > 0);
    }
}

#[test]
fn named_area_items_are_compared_when_consumed() {
    let (mut doc, cascade, child, _) =
        fixture_with_styles("display:grid;grid-template-areas:'a'", "width:1px");
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
    let mut old = tree.layout_style_for_key(parent).unwrap().clone();
    old.grid_template_areas.as_mut().unwrap().areas[0].name = SmolStr::new("wrong");
    let stats = ComparisonStats::default();
    let checked = CheckedStyle::new(
        &old,
        Some(tree.style_view(parent)),
        context(ReadRole::GridContainer),
        &stats,
    );
    let mut areas = checked.grid_template_areas().unwrap();
    assert_eq!(stats.iterator_reads.get(), 0);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| areas.next())).is_err());
}

#[test]
fn actual_taffy_layout_reads_all_four_roles_in_scoped_and_pass_states() {
    let (mut doc, _, grid, _) = fixture_with_styles(
        "display:flex;overflow:hidden;max-width:100px",
        "display:grid;min-width:max-content;grid-template-columns:repeat(2,10px)",
    );
    doc.append_element(Some(grid), "div", LayoutStyle::default(), Some("width:7px"));
    crate::layout::test_support::with_ahem(&mut doc);
    doc.mark_in_document_flags();
    let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    let mut page = raikiri_traits::PageBox::new();
    page.width = 200.0;
    page.height = 100.0;
    crate::layout::page_pipeline::layout_single_page_checked(&mut doc, &cascade, page).unwrap();
    let stats = &doc.style_view_comparisons;
    assert!(
        stats.getter_reads[..4].iter().all(|count| count.get() > 0),
        "{stats:?}"
    );
    assert!(stats.iterator_reads.get() > 0, "{stats:?}");
    assert!(stats.scoped_reads.get() > 0, "{stats:?}");
    assert!(stats.pass_reads.get() > 0, "{stats:?}");
}

#[test]
fn strict_sessions_reject_missing_preparation() {
    let (mut doc, cascade, child, _) = fixture_with_styles("display:flex", "width:31px");
    let tree = LayoutSession::with_cascade(&mut doc, &cascade);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let style = taffy::LayoutFlexboxContainer::get_flexbox_child_style(
            &tree,
            taffy::NodeId::from(child),
        );
        style.size()
    }));
    assert!(result.is_err());
}

#[test]
fn legacy_fixtures_without_a_cascade_are_explicitly_unchecked() {
    let old = LayoutStyle::default();
    let stats = ComparisonStats::default();
    let checked = CheckedStyle::new(&old, None, context(ReadRole::FlexItem), &stats);
    assert_eq!(checked.size(), old.size);
    assert_eq!(stats.getter_reads[ReadRole::FlexItem as usize].get(), 0);
}

#[test]
fn actual_style_reads_count_scoped_and_restored_states() {
    use raikiri_layout_style::{LengthProperty, OwnedPropertyOverride};
    let (mut doc, cascade, child, _) = fixture_with_styles("display:flex", "width:31px");
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
    let original = tree.layout_style_for_key(child).unwrap().size.width;
    tree.with_style_overrides(
        child,
        vec![OwnedPropertyOverride::length(LengthProperty::Width, L::Px(11.0), false).unwrap()],
        |tree| {
            tree.layout_style_for_key_mut(child).unwrap().size.width =
                taffy::Dimension::length(11.0);
            assert_eq!(
                taffy::LayoutFlexboxContainer::get_flexbox_child_style(
                    tree,
                    taffy::NodeId::from(child)
                )
                .size()
                .width,
                taffy::Dimension::length(11.0)
            );
            tree.layout_style_for_key_mut(child).unwrap().size.width = original;
        },
    );
    assert_eq!(
        taffy::LayoutFlexboxContainer::get_flexbox_child_style(&tree, taffy::NodeId::from(child))
            .size()
            .width,
        original
    );
    assert!(tree.style_view_comparisons.scoped_reads.get() > 0);
    assert!(tree.style_view_comparisons.restored_reads.get() > 0);
}

#[test]
fn grid_single_tracks_and_repeat_kind_faults_are_checked_on_consumption() {
    let (mut doc, cascade, child, _) = fixture_with_styles(
        "display:grid;grid-template-columns:10px repeat(2,20px)",
        "width:1px",
    );
    let parent = doc.nodes[child].parent.unwrap();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
    let base = tree.layout_style_for_key(parent).unwrap().clone();
    let view = tree.style_view(parent);
    let stats = ComparisonStats::default();
    let checked = CheckedStyle::new(&base, Some(view), context(ReadRole::GridContainer), &stats);
    assert_eq!(checked.grid_template_columns().unwrap().count(), 2);
    let mut faulty = base.clone();
    faulty.grid_template_columns[0] = base.grid_template_columns[1].clone();
    let checked = CheckedStyle::new(
        &faulty,
        Some(view),
        context(ReadRole::GridContainer),
        &stats,
    );
    let fault = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        checked.grid_template_columns().unwrap().next()
    }));
    let message = panic_text(fault.err().expect("component kind mismatch must panic"));
    assert!(
        message.contains("repeat") && message.contains("single"),
        "{message}"
    );
}
