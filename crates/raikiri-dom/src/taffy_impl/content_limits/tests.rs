//! `min-width`, `max-width`, `min-height` and `max-height` set to an intrinsic
//! sizing keyword take the size the keyword measures (CSS Sizing 3 §3.1.1, §4
//! and §5). The text of the fixtures is `AAAA BBBB` in Ahem at 10px: 40px wide
//! at its longest word, 90px wide on one line.

mod cache_context_tests;
mod scoped_tests;
mod style_view_tests;

use super::{
    ContentLimits, applies, content_percentage_height_basis, replaced_keyword_bounds,
    stretch_fit_width, with_grid_item_content_limits, with_oof_content_limits,
};
use crate::layout::context::LayoutSession;
use crate::layout::ifc::test_support::{block_fixture, block_fixture_with_ahem};
use crate::layout::layout_single_page;
use crate::layout::test_support::{absolute_rect, page_box_800x600, with_ahem};
use crate::node::FormattingContext;
use taffy::{
    AvailableSpace, LayoutInput, LayoutOutput, LengthPercentageAuto, Line, NodeId, RequestedAxis,
    RunMode, Size, SizingMode,
};

/// What the box under test holds.
#[derive(Clone, Copy)]
enum Body {
    /// The text, so the box is a paragraph root.
    Text,
    /// One block of the given size, so the box is a block container of blocks.
    Block(f32, f32),
    /// Two inline-blocks of the given width and 10px height side by side.
    InlineBlocks(f32),
}

/// The border box size `(width, height)` of the box `child_css` styles, laid
/// out after `lead` in a root styled by `root_css`.
fn child_size(root_css: &str, lead: &str, child_css: &str, body: Body) -> (f32, f32) {
    let mut child = 0;
    let fixture = block_fixture(&format!("line-height:10px;{root_css}"), |doc, root| {
        if !lead.is_empty() {
            doc.append_text(root, lead);
        }
        child = doc.append_element(
            Some(root),
            "div",
            crate::LayoutStyle::default(),
            Some(&format!("display:block;{child_css}")),
        );
        match body {
            Body::Text => {
                doc.append_text(child, "AAAA BBBB");
            }
            Body::Block(width, height) => {
                doc.append_element(
                    Some(child),
                    "div",
                    crate::LayoutStyle::default(),
                    Some(&format!("display:block;width:{width}px;height:{height}px")),
                );
            }
            Body::InlineBlocks(width) => {
                for _ in 0..2 {
                    doc.append_element(
                        Some(child),
                        "span",
                        crate::LayoutStyle::default(),
                        Some(&format!("display:inline-block;width:{width}px;height:10px")),
                    );
                }
            }
        }
    });
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    let (_, _, width, height) = absolute_rect(&doc, child);
    (width, height)
}

fn text(root_css: &str, child_css: &str) -> (f32, f32) {
    child_size(root_css, "", child_css, Body::Text)
}

#[test]
fn replaced_keyword_bounds_handles_nonreplaced_missing_invalid_and_sized_nodes() {
    let mut doc = crate::document::Document::new();
    let plain = doc.append_element(None, "div", crate::LayoutStyle::default(), None::<&str>);
    let missing = doc.append_element(None, "img", crate::LayoutStyle::default(), None::<&str>);
    let invalid = doc.append_element(None, "img", crate::LayoutStyle::default(), None::<&str>);
    let sized = doc.append_element(None, "img", crate::LayoutStyle::default(), None::<&str>);
    for (idx, intrinsic) in [
        (invalid, raikiri_traits::IntrinsicBox::new(0.0, 10.0)),
        (sized, raikiri_traits::IntrinsicBox::new(20.0, 10.0)),
    ] {
        if let crate::node::NodeData::Element(element) = &mut doc.nodes[idx].data {
            element.image_intrinsic_size = Some(intrinsic);
        }
    }
    let limits = ContentLimits {
        min: Size {
            width: taffy::Dimension::min_content(),
            height: taffy::Dimension::auto(),
        },
        max: Size {
            width: taffy::Dimension::max_content(),
            height: taffy::Dimension::auto(),
        },
    };
    let inputs = LayoutInput {
        run_mode: RunMode::ComputeSize,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size::NONE,
        known_dimensions_are_definite: Size {
            width: false,
            height: false,
        },
        parent_size: Size::NONE,
        available_space: Size {
            width: AvailableSpace::MaxContent,
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    };
    let session = LayoutSession::new(&mut doc);
    for idx in [plain, missing, invalid] {
        let (width, height) = replaced_keyword_bounds(&session, idx, &limits, &inputs);
        assert_eq!(
            (width.min, width.max, height.min, height.max),
            (None, None, None, None),
            "node {idx} should have no replaced intrinsic keyword bounds"
        );
    }
    let bounds = replaced_keyword_bounds(&session, sized, &limits, &inputs);
    assert_eq!(
        (bounds.0.min, bounds.0.max, bounds.1.min, bounds.1.max),
        (Some(20.0), Some(20.0), Some(10.0), Some(10.0))
    );
}

#[test]
fn replaced_keyword_bounds_follow_the_box_the_ratio_relates() {
    // A 2:1 border box 100px high is 200px wide; the ratio of the content box
    // would give a 90px high content box 180px beside the 40px of padding.
    let mut doc = crate::document::Document::new();
    let style = crate::LayoutStyle {
        aspect_ratio: Some(2.0),
        box_sizing: taffy::BoxSizing::BorderBox,
        size: Size {
            width: taffy::Dimension::auto(),
            height: taffy::Dimension::length(100.0),
        },
        padding: taffy::Rect {
            left: taffy::LengthPercentage::length(0.0),
            right: taffy::LengthPercentage::length(40.0),
            top: taffy::LengthPercentage::length(0.0),
            bottom: taffy::LengthPercentage::length(10.0),
        },
        ..crate::LayoutStyle::default()
    };
    let image = doc.append_element(None, "img", style, None::<&str>);
    if let crate::node::NodeData::Element(element) = &mut doc.nodes[image].data {
        element.image_intrinsic_size = Some(raikiri_traits::IntrinsicBox::new(20.0, 50.0));
    }
    let limits = ContentLimits {
        min: Size {
            width: taffy::Dimension::min_content(),
            height: taffy::Dimension::auto(),
        },
        max: Size::auto(),
    };
    let inputs = LayoutInput {
        run_mode: RunMode::ComputeSize,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size::NONE,
        known_dimensions_are_definite: Size {
            width: false,
            height: false,
        },
        parent_size: Size::NONE,
        available_space: Size {
            width: AvailableSpace::MaxContent,
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    };
    for (content_box_ratio, expected) in [(false, 200.0), (true, 220.0)] {
        doc.nodes[image].ratio_in_content_box = content_box_ratio;
        let session = LayoutSession::new(&mut doc);
        let (width, _) = replaced_keyword_bounds(&session, image, &limits, &inputs);
        assert_eq!(
            width.min,
            Some(expected),
            "content box ratio {content_box_ratio}"
        );
    }
}

#[test]
fn a_min_content_minimum_width_raises_the_width_to_the_content() {
    // The longest word is 40px wide, so the 20px box widens to it and wraps
    // at the space.
    assert_eq!(
        text("width: 200px", "width: 20px; min-width: min-content"),
        (40.0, 20.0)
    );
    // The same with a block container whose content is 100px wide, and with
    // two 50px inline-blocks that may wrap: 50px at the narrowest.
    assert_eq!(
        child_size(
            "width: 200px",
            "",
            "width: 50px; min-width: min-content",
            Body::Block(100.0, 10.0)
        ),
        (100.0, 10.0)
    );
    assert_eq!(
        child_size(
            "width: 200px",
            "",
            "width: 10px; min-width: min-content",
            Body::InlineBlocks(50.0)
        )
        .0,
        50.0
    );
    // Without the keyword the width stands.
    assert_eq!(text("width: 200px", "width: 20px"), (20.0, 20.0));
}

#[test]
fn a_max_content_minimum_width_may_exceed_the_room() {
    // An auto-width box stretches to its 50px parent; `min-width` then lifts
    // it to the 90px of the text on one line.
    assert_eq!(text("width: 50px", "min-width: max-content"), (90.0, 10.0));
    assert_eq!(text("width: 50px", ""), (50.0, 20.0));
    assert_eq!(
        child_size(
            "width: 50px",
            "",
            "min-width: max-content",
            Body::Block(100.0, 10.0)
        ),
        (100.0, 10.0)
    );
}

#[test]
fn a_fit_content_minimum_width_is_the_room_between_min_and_max_content() {
    for (room, expected) in [(200.0, 90.0), (60.0, 60.0), (30.0, 40.0)] {
        assert_eq!(
            text(
                &format!("width: {room}px"),
                "width: 20px; min-width: fit-content"
            )
            .0,
            expected,
            "room {room}px"
        );
    }
}

#[test]
fn a_fit_content_minimum_width_leaves_room_for_the_margins() {
    // 60px less the 10px margin on each side leaves 40px, which is the
    // longest word.
    assert_eq!(
        text(
            "width: 60px",
            "width: 10px; margin: 0 10px; min-width: fit-content"
        )
        .0,
        40.0
    );
    assert_eq!(
        text(
            "width: 80px",
            "width: 10px; margin: 0 10px; min-width: fit-content"
        )
        .0,
        60.0
    );
}

#[test]
fn a_min_content_maximum_width_shrinks_the_width_to_the_content() {
    assert_eq!(
        text("width: 400px", "width: 300px; max-width: min-content"),
        (40.0, 20.0)
    );
    assert_eq!(
        child_size(
            "width: 400px",
            "",
            "width: 300px; max-width: min-content",
            Body::Block(100.0, 10.0)
        ),
        (100.0, 10.0)
    );
    // Auto width is the room, which the maximum width also limits.
    assert_eq!(text("width: 400px", "max-width: min-content"), (40.0, 20.0));
}

#[test]
fn a_max_content_maximum_width_shrinks_the_width_to_the_content() {
    assert_eq!(
        text("width: 400px", "width: 300px; max-width: max-content"),
        (90.0, 10.0)
    );
    assert_eq!(
        text("width: 400px", "width: 50px; max-width: max-content"),
        (50.0, 20.0)
    );
    assert_eq!(text("width: 400px", "max-width: max-content"), (90.0, 10.0));
}

#[test]
fn a_fit_content_maximum_width_is_the_room_between_min_and_max_content() {
    for (room, expected) in [(400.0, 90.0), (60.0, 60.0), (30.0, 40.0)] {
        assert_eq!(
            text(
                &format!("width: {room}px"),
                "width: 300px; max-width: fit-content"
            )
            .0,
            expected,
            "room {room}px"
        );
    }
}

#[test]
fn a_fit_content_function_limit_stands_for_the_room() {
    // 60px lies between the 40px and 90px of the text, whatever the room.
    assert_eq!(
        text("width: 400px", "width: 10px; min-width: fit-content(60px)"),
        (60.0, 20.0)
    );
    assert_eq!(
        text("width: 400px", "width: 300px; max-width: fit-content(60px)"),
        (60.0, 20.0)
    );
    // A limit beyond the content stops at it; one below it stops at the
    // longest word.
    assert_eq!(
        text("width: 400px", "width: 10px; min-width: fit-content(500px)").0,
        90.0
    );
    assert_eq!(
        text("width: 400px", "width: 300px; max-width: fit-content(10px)").0,
        40.0
    );
    // A percentage refers to the width of the container.
    assert_eq!(
        text("width: 120px", "width: 10px; min-width: fit-content(50%)").0,
        60.0
    );
}

#[test]
fn a_keyword_minimum_or_maximum_adds_the_padding_and_border() {
    // The 40px of the longest word, plus 5px of padding on each side.
    for box_sizing in ["content-box", "border-box"] {
        assert_eq!(
            text(
                "width: 200px",
                &format!(
                    "box-sizing: {box_sizing}; width: 10px; padding: 0 5px; \
                     min-width: min-content"
                )
            )
            .0,
            50.0,
            "{box_sizing}"
        );
        assert_eq!(
            text(
                "width: 200px",
                &format!(
                    "box-sizing: {box_sizing}; width: 300px; padding: 0 5px; \
                     max-width: min-content"
                )
            )
            .0,
            50.0,
            "{box_sizing}"
        );
    }
}

#[test]
fn the_minimum_width_wins_over_the_maximum_width() {
    assert_eq!(
        text(
            "width: 400px",
            "width: 200px; min-width: max-content; max-width: min-content"
        )
        .0,
        90.0
    );
    // A length maximum beneath a keyword minimum loses too, and a keyword
    // maximum beneath a length minimum, in a block container and in a
    // paragraph root.
    assert_eq!(
        text(
            "width: 400px",
            "width: 200px; min-width: max-content; max-width: 50px"
        )
        .0,
        90.0
    );
    for lead in ["", "x"] {
        assert_eq!(
            child_size(
                "width: 400px",
                lead,
                "width: 200px; min-width: 100px; max-width: min-content",
                Body::Text
            )
            .0,
            100.0,
            "lead {lead:?}"
        );
    }
}

#[test]
fn a_keyword_width_limit_works_on_a_block_child_of_a_paragraph_root() {
    // A block child of a paragraph root takes the same limits.
    assert_eq!(
        child_size(
            "width: 200px",
            "x",
            "width: 20px; min-width: min-content",
            Body::Text
        ),
        (40.0, 20.0)
    );
    assert_eq!(
        child_size(
            "width: 200px",
            "x",
            "width: 300px; max-width: max-content",
            Body::Text
        ),
        (90.0, 10.0)
    );
    // The child stretches to the 50px of the root and `min-width` lifts it.
    assert_eq!(
        child_size("width: 50px", "x", "min-width: max-content", Body::Text),
        (90.0, 10.0)
    );
}

#[test]
fn a_fit_content_function_limit_without_a_container_width_is_sized_by_the_room() {
    // While the container shrinks to fit, the percentage of the limit has no
    // width to refer to: the child counts as `fit-content` there, and takes
    // half of the 90px the container comes to afterwards.
    let mut inner = 0;
    let fixture = block_fixture("width: fit-content", |doc, root| {
        inner = doc.append_element(
            Some(root),
            "div",
            crate::LayoutStyle::default(),
            Some("display:block;width:10px;min-width:fit-content(50%)"),
        );
        doc.append_text(inner, "AAAA BBBB");
    });
    let root = fixture.root;
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    assert_eq!(absolute_rect(&doc, root).2, 90.0);
    assert_eq!(absolute_rect(&doc, inner).2, 45.0);
}

#[test]
fn a_keyword_width_limit_feeds_the_intrinsic_width_of_its_container() {
    // The container shrinks to fit its only child, which `min-width` lifts
    // from 10px to the 100px of its content (the widths of WPT
    // `fit-content-min-inline-size`).
    let mut inner = 0;
    let fixture = block_fixture("width: fit-content; height: 100px", |doc, root| {
        inner = doc.append_element(
            Some(root),
            "div",
            crate::LayoutStyle::default(),
            Some("display:block;min-width:fit-content;width:10px"),
        );
        for _ in 0..2 {
            doc.append_element(
                Some(inner),
                "span",
                crate::LayoutStyle::default(),
                Some("display:inline-block;width:50px;height:10px"),
            );
        }
    });
    let root = fixture.root;
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    assert_eq!(absolute_rect(&doc, root).2, 100.0);
    assert_eq!(absolute_rect(&doc, inner).2, 100.0);

    // A maximum width leaves the container at the width it limits the child
    // to (WPT `fit-content-max-inline-size`).
    let fixture = block_fixture("width: fit-content; height: 100px", |doc, root| {
        let inner = doc.append_element(
            Some(root),
            "div",
            crate::LayoutStyle::default(),
            Some("display:block;max-width:fit-content;width:200px"),
        );
        for _ in 0..2 {
            doc.append_element(
                Some(inner),
                "span",
                crate::LayoutStyle::default(),
                Some("display:inline-block;width:50px;height:10px"),
            );
        }
    });
    let root = fixture.root;
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    assert_eq!(absolute_rect(&doc, root).2, 100.0);
}

#[test]
fn a_keyword_minimum_height_is_the_height_of_the_content() {
    // WPT `block-size-with-min-or-max-content-2`: the block axis has no
    // content size other than the automatic height.
    for keyword in ["min-content", "max-content", "fit-content"] {
        assert_eq!(
            child_size(
                "width: 200px",
                "",
                &format!("width: 100px; height: 0; min-height: {keyword}"),
                Body::Block(100.0, 100.0)
            ),
            (100.0, 100.0),
            "{keyword}"
        );
    }
    // The text wraps in the 40px the box is given.
    assert_eq!(
        text(
            "width: 200px",
            "width: 40px; height: 0; min-height: max-content"
        ),
        (40.0, 20.0)
    );
    // A function limit is a block-axis keyword as well.
    assert_eq!(
        text(
            "width: 200px",
            "width: 40px; height: 0; min-height: fit-content(5px)"
        ),
        (40.0, 20.0)
    );
}

#[test]
fn a_minimum_height_keyword_sizes_an_orthogonal_block_in_a_vertical_container() {
    assert_eq!(
        child_size(
            "writing-mode:vertical-rl;width:300px;height:200px",
            "",
            "writing-mode:horizontal-tb;width:40px;height:2px;min-height:max-content",
            Body::Block(0.0, 10.0)
        ),
        (40.0, 10.0)
    );
}

#[test]
fn a_maximum_height_keyword_sizes_an_orthogonal_block_in_a_vertical_container() {
    assert_eq!(
        child_size(
            "writing-mode:vertical-rl;width:300px;height:200px",
            "",
            "writing-mode:horizontal-tb;width:40px;height:40px;max-height:min-content",
            Body::Block(0.0, 10.0)
        ),
        (40.0, 10.0)
    );
}

#[test]
fn a_keyword_maximum_height_is_the_height_of_the_content() {
    // WPT `block-size-with-min-or-max-content-3`.
    for keyword in ["min-content", "max-content", "fit-content"] {
        assert_eq!(
            child_size(
                "width: 200px",
                "",
                &format!("width: 100px; height: 200px; max-height: {keyword}"),
                Body::Block(100.0, 100.0)
            ),
            (100.0, 100.0),
            "{keyword}"
        );
    }
    // A height beneath the content stands.
    assert_eq!(
        child_size(
            "width: 200px",
            "",
            "width: 100px; height: 50px; max-height: max-content",
            Body::Block(100.0, 100.0)
        ),
        (100.0, 50.0)
    );
}

#[test]
fn a_keyword_height_limit_measures_the_content_at_the_used_width() {
    // `max-width` narrows the box to 40px before its automatic height, two
    // lines of text, is measured.
    assert_eq!(
        text(
            "width: 200px",
            "max-width: min-content; height: 100px; max-height: max-content"
        ),
        (40.0, 20.0)
    );
}

#[test]
fn a_keyword_height_limit_adds_the_padding_and_border() {
    for box_sizing in ["content-box", "border-box"] {
        assert_eq!(
            child_size(
                "width: 200px",
                "",
                &format!(
                    "box-sizing: {box_sizing}; padding: 5px 0; height: 0; \
                     min-height: max-content"
                ),
                Body::Block(100.0, 100.0)
            )
            .1,
            110.0,
            "{box_sizing}"
        );
    }
}

#[test]
fn border_box_percentage_height_basis_subtracts_edges_once() {
    let mut child = 0;
    let fixture = block_fixture("width:200px;height:200px", |doc, root| {
        child = doc.append_element(
            Some(root),
            "div",
            crate::LayoutStyle::default(),
            Some(
                "box-sizing:border-box;height:100px;min-height:120px;\
                 padding:5px 0;border:2px solid",
            ),
        );
    });
    let mut doc = fixture.doc;
    crate::layout::apply_computed_to_style(&mut doc, &fixture.cascade).expect("bridge");
    let tree = LayoutSession::new(&mut doc);
    let inputs = LayoutInput {
        run_mode: RunMode::ComputeSize,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size::NONE,
        known_dimensions_are_definite: Size {
            width: false,
            height: false,
        },
        parent_size: Size {
            width: Some(200.0),
            height: Some(600.0),
        },
        available_space: Size {
            width: AvailableSpace::Definite(200.0),
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    };

    assert_eq!(
        content_percentage_height_basis(&tree, child, &inputs),
        Some(106.0)
    );
}

#[test]
fn a_keyword_height_limit_applies_to_a_block_in_the_roots_formatting_context() {
    // The block follows text, so it shares the paragraph root's context
    // instead of being laid out on its own.
    assert_eq!(
        child_size(
            "width: 200px",
            "AAAA ",
            "height: 5px; min-height: max-content",
            Body::Block(100.0, 30.0)
        ),
        (200.0, 30.0)
    );
    assert_eq!(
        child_size(
            "width: 200px",
            "AAAA ",
            "height: 100px; max-height: min-content",
            Body::Block(100.0, 30.0)
        ),
        (200.0, 30.0)
    );
}

#[test]
fn floats_and_inline_blocks_honor_intrinsic_min_max_width_and_height() {
    for display in ["float:left", "display:inline-block"] {
        for (property, expected) in [
            ("min-width:min-content", (40.0, 20.0)),
            ("min-width:max-content", (90.0, 10.0)),
            ("min-width:fit-content", (90.0, 10.0)),
            ("min-width:fit-content(60px)", (60.0, 20.0)),
            ("max-width:min-content", (40.0, 20.0)),
            ("max-width:max-content", (90.0, 10.0)),
            ("max-width:fit-content", (90.0, 10.0)),
            ("max-width:fit-content(60px)", (60.0, 20.0)),
        ] {
            let width = if property.starts_with("min-width") {
                "width:10px"
            } else {
                "width:300px"
            };
            assert_eq!(
                child_size(
                    "width:400px",
                    "",
                    &format!("{display};{width};{property}"),
                    Body::Text,
                ),
                expected,
                "{display};{property}"
            );
        }

        for (property, height) in [
            ("min-height:min-content", 20.0),
            ("min-height:max-content", 20.0),
            ("min-height:fit-content", 20.0),
            ("min-height:fit-content(5px)", 20.0),
            ("max-height:min-content", 20.0),
            ("max-height:max-content", 20.0),
            ("max-height:fit-content", 20.0),
            ("max-height:fit-content(5px)", 20.0),
        ] {
            let specified_height = if property.starts_with("min-height") {
                "height:0"
            } else {
                "height:100px"
            };
            assert_eq!(
                child_size(
                    "width:200px",
                    "",
                    &format!("{display};width:40px;{specified_height};{property}"),
                    Body::Text,
                ),
                (40.0, height),
                "{display};{property}"
            );
        }
    }
}

#[test]
fn absolute_boxes_apply_keyword_limits_to_inset_derived_sizes() {
    assert_eq!(
        child_size(
            "position:relative;width:50px;height:10px",
            "",
            "position:absolute;left:10px;right:20px;top:0;min-width:max-content",
            Body::Text,
        ),
        (90.0, 10.0)
    );
    assert_eq!(
        child_size(
            "position:relative;width:200px;height:50px",
            "",
            "position:absolute;left:10px;right:20px;top:0;max-width:min-content",
            Body::Text,
        ),
        (40.0, 20.0)
    );
    assert_eq!(
        child_size(
            "position:relative;width:50px;height:15px",
            "",
            "position:absolute;left:0;right:0;top:0;bottom:5px;width:40px;min-height:max-content",
            Body::Text,
        ),
        (40.0, 20.0)
    );
    assert_eq!(
        child_size(
            "position:relative;width:50px;height:200px",
            "",
            "position:absolute;left:0;right:0;top:0;bottom:0;width:40px;height:100px;max-height:min-content",
            Body::Text,
        ),
        (40.0, 20.0)
    );
}

#[test]
fn absolute_min_content_width_zeros_a_cyclic_percentage_on_a_replaced_child() {
    let mut child = 0;
    let fixture = block_fixture("position:relative;width:100px;height:100px", |doc, root| {
        child = doc.append_element(
            Some(root),
            "div",
            crate::LayoutStyle::default(),
            Some(
                "position:absolute;left:0;right:0;top:0;width:0;min-width:min-content;\
                 border:3px solid;padding:2px",
            ),
        );
        let canvas = doc.append_element(
            Some(child),
            "canvas",
            crate::LayoutStyle::default(),
            Some("display:block;width:100%"),
        );
        doc.set_element_attribute(canvas, "width", "20")
            .expect("set canvas width");
        doc.set_element_attribute(canvas, "height", "10")
            .expect("set canvas height");
    });
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    assert_eq!(absolute_rect(&doc, child).2, 10.0);
}

#[test]
fn positioned_intrinsic_height_limits_keep_the_authored_percentage_basis_for_children() {
    let mut children = Vec::new();
    let fixture = block_fixture("position:relative;width:100px;height:600px", |doc, root| {
        for (height_limit, preferred_height, expected) in [
            ("min-height:min-content", "height:0", 10.0),
            ("max-height:min-content", "height:500px", 510.0),
        ] {
            let child = doc.append_element(
                Some(root),
                "div",
                crate::LayoutStyle::default(),
                Some(&format!(
                    "position:absolute;left:0;top:0;width:10px;{preferred_height};\
                     {height_limit};border:3px solid;padding:2px"
                )),
            );
            let canvas = doc.append_element(
                Some(child),
                "canvas",
                crate::LayoutStyle::default(),
                Some("display:block;height:100%"),
            );
            doc.set_element_attribute(canvas, "width", "10")
                .expect("set canvas width");
            doc.set_element_attribute(canvas, "height", "20")
                .expect("set canvas height");
            children.push((child, expected));
        }
    });
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    for (child, expected) in children {
        assert_eq!(absolute_rect(&doc, child).3, expected);
    }
}

#[test]
fn positioned_height_keywords_measure_at_the_width_after_width_limits() {
    for (position, root_css, insets, shrink_root_css, shrink_insets) in [
        (
            "absolute",
            "position:relative;width:50px;height:100px",
            "left:0;right:0",
            "position:relative;width:200px;height:100px",
            "left:0;right:0",
        ),
        (
            "fixed",
            "width:50px;height:100px",
            "left:0;right:750px",
            "width:200px;height:100px",
            "left:0;right:600px",
        ),
    ] {
        assert_eq!(
            child_size(
                root_css,
                "",
                &format!(
                    "position:{position};{insets};top:0;width:auto;height:0;\
                     min-width:max-content;min-height:max-content"
                ),
                Body::Text,
            ),
            (90.0, 10.0),
            "position:{position}; width grows before height measurement"
        );
        assert_eq!(
            child_size(
                shrink_root_css,
                "",
                &format!(
                    "position:{position};{shrink_insets};top:0;width:auto;height:100px;\
                     max-width:min-content;max-height:min-content"
                ),
                Body::Text,
            ),
            (40.0, 20.0),
            "position:{position}; width shrinks before height measurement"
        );
        assert_eq!(
            child_size(
                shrink_root_css,
                "",
                &format!(
                    "position:{position};{shrink_insets};top:0;width:auto;height:100px;\
                     min-width:100px;max-width:min-content;max-height:min-content"
                ),
                Body::Text,
            ),
            (100.0, 10.0),
            "position:{position}; numeric minimum wins while measuring height"
        );
    }
}

#[test]
fn normal_block_min_height_keywords_treat_cyclic_percentage_children_as_auto() {
    let mut article = 0;
    let fixture = block_fixture("width:300px", |doc, root| {
        article = doc.append_element(
            Some(root),
            "div",
            crate::LayoutStyle::default(),
            Some("display:block;height:100px;min-height:min-content"),
        );
        let aside = doc.append_element(
            Some(article),
            "div",
            crate::LayoutStyle::default(),
            Some("display:block;height:50%"),
        );
        doc.append_element(
            Some(aside),
            "div",
            crate::LayoutStyle::default(),
            Some("display:block;height:150px"),
        );
        doc.append_element(
            Some(article),
            "div",
            crate::LayoutStyle::default(),
            Some("display:block;height:30px"),
        );
    });
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    assert_eq!(absolute_rect(&doc, article).3, 180.0);
}

#[test]
fn fixed_boxes_apply_keyword_limits_to_inset_derived_sizes() {
    assert_eq!(
        child_size(
            "width:50px;height:10px",
            "",
            "position:fixed;left:10px;right:780px;top:0;min-width:max-content",
            Body::Text,
        ),
        (90.0, 10.0)
    );
    assert_eq!(
        child_size(
            "width:200px;height:50px",
            "",
            "position:fixed;left:10px;right:20px;top:0;max-width:min-content",
            Body::Text,
        ),
        (40.0, 20.0)
    );
    assert_eq!(
        child_size(
            "width:50px;height:15px",
            "",
            "position:fixed;left:0;right:0;top:0;bottom:590px;width:40px;min-height:max-content",
            Body::Text,
        ),
        (40.0, 20.0)
    );
    assert_eq!(
        child_size(
            "width:50px;height:200px",
            "",
            "position:fixed;left:0;right:0;top:0;bottom:0;width:40px;height:100px;max-height:min-content",
            Body::Text,
        ),
        (40.0, 20.0)
    );
}

#[test]
fn positioned_width_keywords_use_inset_room_and_containing_block_basis() {
    for (position, min_root, min_insets, max_root, max_insets) in [
        (
            "absolute",
            "position:relative;width:50px;height:20px",
            "left:10px;right:20px",
            "position:relative;width:400px;height:20px",
            "left:10px;right:20px",
        ),
        (
            "fixed",
            "width:50px;height:20px",
            "left:10px;right:780px",
            "width:400px;height:20px",
            "left:10px;right:20px",
        ),
    ] {
        for (property, expected) in [
            ("min-width:min-content", (40.0, 20.0)),
            ("min-width:max-content", (90.0, 10.0)),
            ("min-width:fit-content", (40.0, 20.0)),
            ("min-width:fit-content(60px)", (60.0, 20.0)),
        ] {
            assert_eq!(
                child_size(
                    min_root,
                    "",
                    &format!("position:{position};{min_insets};top:0;width:auto;{property}"),
                    Body::Text,
                ),
                expected,
                "position:{position};{property}"
            );
        }
        for (property, expected) in [
            ("max-width:min-content", (40.0, 20.0)),
            ("max-width:max-content", (90.0, 10.0)),
            ("max-width:fit-content", (90.0, 10.0)),
            ("max-width:fit-content(60px)", (60.0, 20.0)),
        ] {
            assert_eq!(
                child_size(
                    max_root,
                    "",
                    &format!("position:{position};{max_insets};top:0;width:auto;{property}"),
                    Body::Text,
                ),
                expected,
                "position:{position};{property}"
            );
        }
    }

    // Percentage fit-content limits use the containing block width even when
    // opposing insets leave a much narrower inset-modified containing block.
    assert_eq!(
        child_size(
            "position:relative;width:200px;height:20px",
            "",
            "position:absolute;left:90px;right:90px;top:0;min-width:fit-content(50%)",
            Body::Text,
        ),
        (90.0, 10.0)
    );
    assert_eq!(
        child_size(
            "width:50px;height:20px",
            "",
            "position:fixed;left:390px;right:390px;top:0;min-width:fit-content(50%)",
            Body::Text,
        ),
        (90.0, 10.0)
    );
}

#[test]
fn positioned_height_keywords_measure_automatic_content_height() {
    for (position, min_root, max_root, min_insets, max_insets) in [
        (
            "absolute",
            "position:relative;width:50px;height:15px",
            "position:relative;width:50px;height:200px",
            "top:0;bottom:5px",
            "top:0;bottom:0",
        ),
        (
            "fixed",
            "width:50px;height:15px",
            "width:50px;height:200px",
            "top:0;bottom:590px",
            "top:0;bottom:0",
        ),
    ] {
        for property in [
            "min-height:min-content",
            "min-height:max-content",
            "min-height:fit-content",
            "min-height:fit-content(5px)",
        ] {
            assert_eq!(
                child_size(
                    min_root,
                    "",
                    &format!(
                        "position:{position};left:0;right:0;{min_insets};width:40px;height:0;{property}"
                    ),
                    Body::Text,
                ),
                (40.0, 20.0),
                "position:{position};{property}"
            );
        }
        for property in [
            "max-height:min-content",
            "max-height:max-content",
            "max-height:fit-content",
            "max-height:fit-content(5px)",
        ] {
            assert_eq!(
                child_size(
                    max_root,
                    "",
                    &format!(
                        "position:{position};left:0;right:0;{max_insets};width:40px;height:100px;{property}"
                    ),
                    Body::Text,
                ),
                (40.0, 20.0),
                "position:{position};{property}"
            );
        }
    }
}

#[test]
fn positioned_keyword_limits_keep_the_minimum_stronger_than_the_maximum() {
    for position in ["absolute", "fixed"] {
        assert_eq!(
            child_size(
                "position:relative;width:200px;height:100px",
                "",
                &format!(
                    "position:{position};left:10px;right:20px;top:0;\
                     min-width:100px;max-width:min-content"
                ),
                Body::Text,
            )
            .0,
            100.0,
            "position:{position};length minimum beats keyword maximum"
        );
        assert_eq!(
            child_size(
                "position:relative;width:200px;height:100px",
                "",
                &format!(
                    "position:{position};left:10px;right:20px;top:0;\
                     min-width:max-content;max-width:50px"
                ),
                Body::Text,
            )
            .0,
            90.0,
            "position:{position};keyword minimum beats length maximum"
        );
    }
}

#[test]
fn positioned_keyword_limits_restore_the_bridged_style_after_layout() {
    let mut child = 0;
    let fixture =
        block_fixture_with_ahem("position:relative;width:50px;height:15px", |doc, root| {
            child = doc.append_element(
                Some(root),
                "div",
                crate::LayoutStyle::default(),
                Some(
                    "position:absolute;left:10px;right:20px;top:0;\
                     min-width:max-content;max-height:min-content",
                ),
            );
            doc.append_text(child, "AAAA BBBB");
        });
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    let style = doc.layout_node_with_style(child).unwrap().style;
    assert!(style.min_size.width.is_auto());
    assert!(style.max_size.height.is_auto());
}

#[test]
fn absolute_keyword_limits_apply_inside_inline_containing_blocks() {
    let mut children = Vec::new();
    let fixture = block_fixture_with_ahem("width:200px;line-height:10px", |doc, root| {
        let inline = doc.append_element(
            Some(root),
            "span",
            crate::LayoutStyle::default(),
            Some("display:inline;position:relative"),
        );
        doc.append_text(inline, "X");
        for style in [
            "left:0;right:0;top:0;min-width:max-content",
            "left:0;right:0;top:0;width:100px;max-width:min-content",
            "left:0;right:0;top:0;width:90px;height:0;min-height:max-content",
            "left:0;right:0;top:0;width:40px;height:100px;max-height:min-content",
            "left:0;right:0;top:0;width:100px;min-width:max-content;max-width:50px",
            "left:0;right:0;top:0;width:40px;min-width:100px;max-width:min-content",
            "left:0;right:0;top:0;width:90px;height:100px;min-height:max-content;max-height:5px",
            "left:0;right:0;top:0;width:40px;height:40px;min-height:100px;max-height:min-content",
            "left:0;right:0;top:0;width:40px;min-width:100px;height:100px;max-height:min-content",
        ] {
            let child = doc.append_element(
                Some(inline),
                "span",
                crate::LayoutStyle::default(),
                Some(&format!("display:inline;position:absolute;{style}")),
            );
            doc.append_text(child, "AAAA BBBB");
            children.push(child);
        }
    });
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    let rect = |index| absolute_rect(&doc, children[index]);
    assert_eq!(rect(0).2, 90.0, "min-width:max-content");
    assert_eq!(rect(1).2, 40.0, "max-width:min-content");
    assert_eq!(rect(2).3, 10.0, "min-height:max-content");
    assert_eq!(rect(3).3, 20.0, "max-height:min-content");
    assert_eq!(
        rect(4).2,
        90.0,
        "intrinsic min-width beats numeric max-width"
    );
    assert_eq!(
        rect(5).2,
        100.0,
        "numeric min-width beats intrinsic max-width"
    );
    assert_eq!(
        rect(6).3,
        10.0,
        "intrinsic min-height beats numeric max-height"
    );
    assert_eq!(
        rect(7).3,
        100.0,
        "numeric min-height beats intrinsic max-height"
    );
    assert_eq!(
        (rect(8).2, rect(8).3),
        (100.0, 10.0),
        "numeric min-width clamps the known width before measuring intrinsic height"
    );
}

#[test]
fn nested_oof_content_limits_do_not_grow_exponentially_by_depth() {
    let depth = 12;
    let mut candidates = Vec::new();
    let fixture = block_fixture_with_ahem("width:200px;height:200px", |doc, root| {
        let mut parent = root;
        for _ in 0..depth {
            parent = doc.append_element(
                Some(parent),
                "div",
                crate::LayoutStyle::default(),
                Some("display:block;position:absolute;min-width:max-content"),
            );
            candidates.push(NodeId::from(parent));
        }
        doc.append_text(parent, "AAAA BBBB");
    });
    let mut doc = fixture.doc;
    let mut session = LayoutSession::new(&mut doc);
    let mut dispatches = 0;

    fn dispatch_nested(
        tree: &mut LayoutSession<'_>,
        candidates: &[NodeId],
        dispatches: &mut usize,
    ) {
        *dispatches += 1;
        let Some((&candidate, rest)) = candidates.split_first() else {
            return;
        };
        with_oof_content_limits(tree, Some(&[candidate]), &[], |tree| {
            dispatch_nested(tree, rest, dispatches);
        });
    }

    with_oof_content_limits(&mut session, None, &[], |tree| {
        dispatch_nested(tree, &candidates, &mut dispatches);
    });

    // The nested wrapper may still visit each descendant during each level's
    // sizing pass, but it must not repeat the full sizing protocol recursively.
    assert!(
        dispatches <= 4 * depth * depth,
        "nested OOF layout dispatched {dispatches} times at depth {depth}"
    );
}

#[test]
fn nested_oof_prepasses_do_not_reuse_an_unconstrained_descendant_cache() {
    let mut descendant = 0;
    let fixture =
        block_fixture_with_ahem("position:relative;width:200px;height:200px", |doc, root| {
            let parent = doc.append_element(
                Some(root),
                "div",
                crate::LayoutStyle::default(),
                Some(
                    "display:block;position:absolute;left:0;top:0;width:100px;height:100px;\
                 min-width:max-content",
                ),
            );
            doc.append_text(parent, "AAAAAAAAAA");
            let intermediate = doc.append_element(
                Some(parent),
                "div",
                crate::LayoutStyle::default(),
                Some("display:block;position:absolute;left:0;top:0;width:50px;height:50px"),
            );
            descendant = doc.append_element(
                Some(intermediate),
                "div",
                crate::LayoutStyle::default(),
                Some("display:block;position:absolute;left:0;top:0;width:0;min-width:max-content"),
            );
            doc.append_text(descendant, "AAAA BBBB");
        });
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");

    assert_eq!(absolute_rect(&doc, descendant).2, 90.0);
}

#[test]
fn the_stretch_fit_width_is_the_containing_block_less_margins_within_the_room() {
    let mut child = 0;
    let fixture = block_fixture("width: 200px", |doc, root| {
        child = doc.append_element(
            Some(root),
            "div",
            crate::LayoutStyle::default(),
            Some("display:block;margin:0 10px 0 auto"),
        );
    });
    let mut doc = fixture.doc;
    let session = LayoutSession::new(&mut doc);
    let inputs = |basis: Option<f32>, room: f32| LayoutInput {
        run_mode: RunMode::ComputeSize,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size::NONE,
        known_dimensions_are_definite: Size {
            width: false,
            height: false,
        },
        parent_size: Size {
            width: basis,
            height: None,
        },
        available_space: Size {
            width: AvailableSpace::Definite(room),
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    };
    // An `auto` margin counts for nothing: 200px less the 10px on the right.
    assert_eq!(
        stretch_fit_width(&session, child, &inputs(Some(200.0), 500.0), 500.0),
        190.0
    );
    // A room smaller than that is all the box gets.
    assert_eq!(
        stretch_fit_width(&session, child, &inputs(Some(200.0), 120.0), 120.0),
        120.0
    );
    // Without a containing block width, the room is.
    assert_eq!(
        stretch_fit_width(&session, child, &inputs(None, 120.0), 120.0),
        120.0
    );
}

#[test]
fn boxes_with_keyword_limits_nested_deeply_are_measured_once_per_level() {
    let depth = 12;
    let mut first = 0;
    let fixture = block_fixture_with_ahem("line-height: 10px; width: 200px", |doc, root| {
        let mut parent = root;
        for _ in 0..depth {
            parent = doc.append_element(
                Some(parent),
                "div",
                crate::LayoutStyle::default(),
                Some(
                    "display:block;min-width:min-content;max-width:max-content;\
                     min-height:min-content",
                ),
            );
            if first == 0 {
                first = parent;
            }
        }
        doc.append_text(parent, "AAAA BBBB");
    });
    let mut doc = fixture.doc;
    let mut session = LayoutSession::new(&mut doc);
    let inputs = LayoutInput {
        run_mode: RunMode::PerformLayout,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size {
            width: Some(200.0),
            height: None,
        },
        known_dimensions_are_definite: Size {
            width: true,
            height: false,
        },
        parent_size: Size {
            width: Some(200.0),
            height: None,
        },
        available_space: Size {
            width: AvailableSpace::Definite(200.0),
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    };
    // Measuring every level again for each measure of the level above would
    // take longer than the test run does.
    let output = session.compute_child_layout_with_block_ctx(NodeId::from(first), inputs, None);
    assert_eq!(output.size.width, 90.0);
    assert_eq!(session.context.content_limit_depth, 0);
    assert!(session.context.content_limit_measures.len() <= 16 * depth);
}

#[test]
fn limits_are_resolved_for_block_inline_and_flex_grid_boxes_that_are_not_being_measured() {
    let mut child = 0;
    let fixture = block_fixture("width: 200px", |doc, root| {
        child = doc.append_element(
            Some(root),
            "div",
            crate::LayoutStyle::default(),
            Some("display:block;min-width:min-content"),
        );
    });
    let mut doc = fixture.doc;
    let mut session = LayoutSession::new(&mut doc);
    let limits = session.nodes[child].content_limits;
    assert_ne!(limits, super::ContentLimits::NONE);
    for context in [
        FormattingContext::Block,
        FormattingContext::Inline,
        FormattingContext::Flex,
        FormattingContext::Grid,
    ] {
        session.nodes[child].formatting_context = context;
        assert!(applies(&session, child, &limits));
    }
    // A box laid out by another algorithm keeps the lengths of its style.
    session.nodes[child].formatting_context = FormattingContext::Multicol;
    assert!(!applies(&session, child, &limits));
    // So does a box that is being measured without its limits.
    session.nodes[child].formatting_context = FormattingContext::Block;
    session.context.content_limit_probes.push(child);
    assert!(!applies(&session, child, &limits));
    // And a box without any.
    session.context.content_limit_probes.clear();
    assert!(!applies(&session, child, &super::ContentLimits::NONE));
}

fn flex_item_size(direction: &str, root_css: &str, item_css: &str) -> (f32, f32) {
    child_size(
        &format!("display:flex;flex-direction:{direction};align-items:flex-start;{root_css}"),
        "",
        &format!("flex:none;{item_css}"),
        Body::Text,
    )
}

fn canvas_intrinsic_contribution_size(container_display: &str, canvas_css: &str) -> (f32, f32) {
    let (mut outer, mut canvas) = (0, 0);
    let fixture = block_fixture("width:200px", |doc, root| {
        outer = doc.append_element(
            Some(root),
            "div",
            crate::LayoutStyle::default(),
            Some(&format!(
                "display:{container_display};width:max-content;border:5px solid;margin:5px"
            )),
        );
        canvas = doc.append_element(
            Some(outer),
            "canvas",
            crate::LayoutStyle::default(),
            Some(&format!("display:block;width:max-content;{canvas_css}")),
        );
        doc.set_element_attribute(canvas, "width", "50")
            .expect("set canvas width");
        doc.set_element_attribute(canvas, "height", "50")
            .expect("set canvas height");
    });
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    let rect = absolute_rect(&doc, outer);
    (rect.2, rect.3)
}

#[test]
fn replaced_min_and_max_height_keywords_contribute_to_intrinsic_inline_size() {
    for display in [
        "block",
        "flex;flex-direction:row",
        "flex;flex-direction:column",
        "grid",
    ] {
        for canvas_css in [
            "height:0;min-height:max-content",
            "height:100px;max-height:max-content",
        ] {
            assert_eq!(
                canvas_intrinsic_contribution_size(display, canvas_css),
                (60.0, 60.0),
                "display:{display};{canvas_css}"
            );
        }
    }
}

#[test]
fn an_out_of_flow_replaced_min_height_keyword_uses_its_inset_width() {
    let mut canvas = 0;
    let fixture = block_fixture("position:relative;width:60px;height:60px", |doc, root| {
        canvas = doc.append_element(
            Some(root),
            "canvas",
            crate::LayoutStyle::default(),
            Some(
                "position:absolute;left:5px;right:5px;top:0;width:auto;height:0;\
                 min-height:max-content",
            ),
        );
        doc.set_element_attribute(canvas, "width", "50")
            .expect("set canvas width");
        doc.set_element_attribute(canvas, "height", "50")
            .expect("set canvas height");
    });
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    let rect = absolute_rect(&doc, canvas);
    assert_eq!((rect.2, rect.3), (50.0, 50.0));
}

#[test]
fn flex_items_honor_intrinsic_min_width_values() {
    for (keyword, width, height) in [
        ("min-content", 40.0, 20.0),
        ("max-content", 90.0, 10.0),
        ("fit-content", 60.0, 20.0),
        ("fit-content(60px)", 60.0, 20.0),
    ] {
        assert_eq!(
            flex_item_size(
                "row",
                "width:60px;height:100px",
                &format!("width:10px;min-width:{keyword}"),
            ),
            (width, height),
            "min-width:{keyword}"
        );
    }
    assert_eq!(
        flex_item_size(
            "row",
            "width:120px;height:100px",
            "width:10px;min-width:fit-content(50%)",
        ),
        (60.0, 20.0)
    );
    // Width is the cross axis of a column flex container.
    assert_eq!(
        flex_item_size(
            "column",
            "width:120px;height:100px",
            "width:10px;min-width:max-content",
        ),
        (90.0, 10.0)
    );
}

#[test]
fn flex_items_honor_intrinsic_max_width_values() {
    for (keyword, width, height) in [
        ("min-content", 40.0, 20.0),
        ("max-content", 90.0, 10.0),
        ("fit-content", 60.0, 20.0),
        ("fit-content(60px)", 60.0, 20.0),
    ] {
        assert_eq!(
            flex_item_size(
                "row",
                "width:60px;height:100px",
                &format!("width:300px;max-width:{keyword}"),
            ),
            (width, height),
            "max-width:{keyword}"
        );
    }
    assert_eq!(
        flex_item_size(
            "row",
            "width:120px;height:100px",
            "width:300px;max-width:fit-content(50%)",
        ),
        (60.0, 20.0)
    );
    // Width is the cross axis of a column flex container.
    assert_eq!(
        flex_item_size(
            "column",
            "width:120px;height:100px",
            "width:300px;max-width:min-content",
        ),
        (40.0, 20.0)
    );
}

#[test]
fn flex_items_honor_intrinsic_min_height_values_on_both_axes() {
    for (direction, root_css) in [
        ("column", "width:120px;height:100px"),
        ("row", "width:60px;height:100px"),
    ] {
        for keyword in [
            "min-content",
            "max-content",
            "fit-content",
            "fit-content(5px)",
        ] {
            assert_eq!(
                flex_item_size(
                    direction,
                    root_css,
                    &format!("width:40px;height:0;min-height:{keyword}"),
                ),
                (40.0, 20.0),
                "direction:{direction};min-height:{keyword}"
            );
        }
    }
}

#[test]
fn flex_items_honor_intrinsic_max_height_values_on_both_axes() {
    for (direction, root_css) in [
        ("column", "width:120px;height:100px"),
        ("row", "width:60px;height:100px"),
    ] {
        for keyword in [
            "min-content",
            "max-content",
            "fit-content",
            "fit-content(5px)",
        ] {
            assert_eq!(
                flex_item_size(
                    direction,
                    root_css,
                    &format!("width:40px;height:100px;max-height:{keyword}"),
                ),
                (40.0, 20.0),
                "direction:{direction};max-height:{keyword}"
            );
        }
    }
}

#[test]
fn flex_item_intrinsic_min_width_preserves_auto_and_border_box_sizes() {
    let default_auto = flex_item_size("row", "width:60px;height:100px", "width:10px");
    assert_eq!(
        flex_item_size(
            "row",
            "width:60px;height:100px",
            "width:10px;min-width:auto"
        ),
        default_auto
    );
    assert_eq!(
        flex_item_size(
            "row",
            "width:60px;height:100px",
            "box-sizing:border-box;width:10px;padding:0 5px;min-width:min-content",
        ),
        (50.0, 20.0)
    );
}

#[test]
fn flex_item_intrinsic_min_max_styles_are_restored_after_layout() {
    let mut child = 0;
    let fixture = block_fixture_with_ahem(
        "display:flex;width:60px;height:100px;align-items:flex-start",
        |doc, root| {
            child = doc.append_element(
                Some(root),
                "div",
                crate::LayoutStyle::default(),
                Some("display:block;flex:none;width:10px;min-width:min-content;max-height:max-content"),
            );
            doc.append_text(child, "AAAA BBBB");
        },
    );
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    let style = doc.layout_node_with_style(child).unwrap().style;
    assert!(style.min_size.width.is_auto());
    assert!(style.max_size.height.is_auto());
}

#[test]
fn an_explicit_intrinsic_flex_item_minimum_changes_sibling_space_distribution() {
    let (mut first, mut second) = (0, 0);
    let fixture = block_fixture_with_ahem(
        "display:flex;width:150px;height:100px;align-items:flex-start",
        |doc, root| {
            first = doc.append_element(
                Some(root),
                "div",
                crate::LayoutStyle::default(),
                Some("display:block;flex:1 1 auto;width:100px;min-width:max-content"),
            );
            doc.append_text(first, "AAAA BBBB");
            second = doc.append_element(
                Some(root),
                "div",
                crate::LayoutStyle::default(),
                Some("display:block;flex:1 1 auto;width:100px"),
            );
            doc.append_text(second, "A");
        },
    );
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    let first_rect = absolute_rect(&doc, first);
    let second_rect = absolute_rect(&doc, second);
    assert_eq!((first_rect.0, first_rect.2), (0.0, 90.0));
    assert_eq!((second_rect.0, second_rect.2), (90.0, 60.0));
}

fn grid_item_size(root_css: &str, item_css: &str) -> (f32, f32) {
    child_size(
        &format!(
            "display:grid;width:60px;height:100px;grid-template-columns:60px;\
             grid-template-rows:100px;\
             justify-items:start;align-items:start;{root_css}"
        ),
        "",
        &format!("grid-column:1;grid-row:1;{item_css}"),
        Body::Text,
    )
}

#[test]
fn grid_items_honor_intrinsic_min_width_values_on_fixed_and_auto_tracks() {
    for (keyword, width, height) in [
        ("min-content", 40.0, 20.0),
        ("max-content", 90.0, 10.0),
        ("fit-content", 60.0, 20.0),
        ("fit-content(60px)", 60.0, 20.0),
    ] {
        assert_eq!(
            grid_item_size("", &format!("width:10px;min-width:{keyword}")),
            (width, height),
            "min-width:{keyword}"
        );
    }
    assert_eq!(
        grid_item_size(
            "width:120px;grid-template-columns:120px",
            "width:10px;min-width:fit-content(50%)",
        ),
        (60.0, 20.0)
    );
    assert_eq!(
        grid_item_size(
            "width:200px;grid-template-columns:100px 100px",
            "width:10px;min-width:fit-content(50%)",
        ),
        (50.0, 20.0),
        "percentage fit-content uses the assigned 100px grid area as its basis"
    );
    assert_eq!(
        grid_item_size(
            "",
            "box-sizing:border-box;width:10px;padding:0 5px;min-width:min-content",
        ),
        (50.0, 20.0)
    );

    let mut child = 0;
    let fixture = block_fixture_with_ahem(
        "display:grid;width:60px;grid-template-columns:auto;grid-template-rows:100px;\
         justify-items:start;align-items:start",
        |doc, root| {
            child = doc.append_element(
                Some(root),
                "div",
                crate::LayoutStyle::default(),
                Some("display:block;grid-column:1;grid-row:1;width:10px;min-width:max-content"),
            );
            doc.append_text(child, "AAAA BBBB");
        },
    );
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    assert_eq!(absolute_rect(&doc, child).2, 90.0);
}

#[test]
fn grid_percentage_fit_content_uses_container_basis_without_track_details() {
    let mut child = 0;
    let fixture = block_fixture_with_ahem(
        "display:grid;width:200px;height:100px;grid-template-columns:100px 100px;\
         grid-template-rows:100px;justify-items:start;align-items:start",
        |doc, root| {
            child = doc.append_element(
                Some(root),
                "div",
                crate::LayoutStyle::default(),
                Some("grid-column:1;grid-row:1;width:10px;min-width:fit-content(50%)"),
            );
            doc.append_text(child, "AAAA BBBB");
        },
    );
    let mut doc = fixture.doc;
    let mut tree = LayoutSession::new(&mut doc);
    let parent_size = Size {
        width: Some(200.0),
        height: Some(100.0),
    };
    let inputs = LayoutInput {
        run_mode: RunMode::ComputeSize,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size::NONE,
        known_dimensions_are_definite: Size {
            width: false,
            height: false,
        },
        parent_size,
        available_space: Size {
            width: AvailableSpace::Definite(200.0),
            height: AvailableSpace::Definite(100.0),
        },
        vertical_margins_are_collapsible: Line::FALSE,
    };
    let mut dispatches = 0;
    let mut installed_min_width = None;
    with_grid_item_content_limits(
        &mut tree,
        NodeId::from(fixture.root),
        inputs,
        parent_size,
        |tree, _, _| {
            dispatches += 1;
            if dispatches == 2 {
                installed_min_width = Some(
                    tree.layout_node_with_style(child)
                        .unwrap()
                        .style
                        .min_size
                        .width,
                );
            }
            LayoutOutput::from_outer_size(Size {
                width: 200.0,
                height: 100.0,
            })
        },
    );

    assert_eq!(dispatches, 2, "preliminary and final grid passes");
    assert_eq!(
        installed_min_width,
        Some(LengthPercentageAuto::length(90.0)),
        "without track details, 50% uses the 200px grid container basis"
    );
}

#[test]
fn flex_grid_boxes_do_not_preserve_logical_min_block_size_keywords() {
    for display in ["flex", "grid"] {
        let mut child = 0;
        let fixture = block_fixture(
            &format!("display:{display};width:100px;height:100px;min-block-size:min-content"),
            |doc, root| {
                child = doc.append_element(
                    Some(root),
                    "div",
                    crate::LayoutStyle::default(),
                    Some("min-block-size:min-content"),
                );
            },
        );
        let mut doc = fixture.doc;
        let tree = LayoutSession::new(&mut doc);
        assert_eq!(
            tree.nodes[fixture.root].content_limits,
            super::ContentLimits::NONE,
            "{display} container logical min-block-size is outside this feature"
        );
        assert_eq!(
            tree.nodes[child].content_limits,
            super::ContentLimits::NONE,
            "{display} item logical min-block-size is outside this feature"
        );
    }
}

#[test]
fn flex_grid_items_keep_physical_min_width_with_logical_min_block_size() {
    let flex_item = flex_item_size(
        "row",
        "width:200px;height:100px",
        "width:10px;min-width:min-content;min-block-size:min-content",
    );
    assert_eq!(flex_item.0, 40.0, "flex item width");

    let grid_item = grid_item_size(
        "width:200px;grid-template-columns:200px",
        "width:10px;min-width:min-content;min-block-size:min-content",
    );
    assert_eq!(grid_item.0, 40.0, "grid item width");
}

#[test]
fn grid_items_honor_intrinsic_max_width_values() {
    for (keyword, width, height) in [
        ("min-content", 40.0, 20.0),
        ("max-content", 90.0, 10.0),
        ("fit-content", 60.0, 20.0),
        ("fit-content(60px)", 60.0, 20.0),
    ] {
        assert_eq!(
            grid_item_size("", &format!("width:300px;max-width:{keyword}")),
            (width, height),
            "max-width:{keyword}"
        );
    }
    assert_eq!(
        grid_item_size(
            "width:120px;grid-template-columns:120px",
            "width:300px;max-width:fit-content(50%)",
        ),
        (60.0, 20.0)
    );
}

#[test]
fn grid_items_honor_intrinsic_min_height_values() {
    for keyword in [
        "min-content",
        "max-content",
        "fit-content",
        "fit-content(5px)",
    ] {
        assert_eq!(
            grid_item_size("", &format!("width:40px;height:0;min-height:{keyword}")),
            (40.0, 20.0),
            "min-height:{keyword}"
        );
    }
}

#[test]
fn grid_items_honor_intrinsic_max_height_values() {
    for keyword in [
        "min-content",
        "max-content",
        "fit-content",
        "fit-content(5px)",
    ] {
        assert_eq!(
            grid_item_size("", &format!("width:40px;height:100px;max-height:{keyword}"),),
            (40.0, 20.0),
            "max-height:{keyword}"
        );
    }
}

fn intrinsic_container_size(display: &str, root_css: &str) -> (f32, f32) {
    let display_css = match display {
        "flex" => "display:flex;align-items:flex-start",
        "grid" => {
            "display:grid;grid-template-columns:auto;grid-template-rows:auto;\
             justify-items:start;align-items:start"
        }
        _ => panic!("unsupported container display: {display}"),
    };
    let item_css = match display {
        "flex" => "display:block;flex:1 1 auto",
        "grid" => "display:block;grid-column:1;grid-row:1",
        _ => unreachable!(),
    };
    let fixture = block_fixture_with_ahem(&format!("{display_css};{root_css}"), |doc, root| {
        let item = doc.append_element(
            Some(root),
            "div",
            crate::LayoutStyle::default(),
            Some(item_css),
        );
        doc.append_text(item, "AAAA BBBB");
    });
    let root = fixture.root;
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    let rect = absolute_rect(&doc, root);
    (rect.2, rect.3)
}

#[test]
fn flex_and_grid_containers_honor_intrinsic_min_max_width_values() {
    for display in ["flex", "grid"] {
        for (css, width, height) in [
            ("width:20px;min-width:min-content", 40.0, 20.0),
            ("width:20px;min-width:max-content", 90.0, 10.0),
            ("width:20px;min-width:fit-content", 90.0, 10.0),
            ("width:20px;min-width:fit-content(60px)", 60.0, 20.0),
            ("width:400px;max-width:min-content", 40.0, 20.0),
            ("width:400px;max-width:max-content", 90.0, 10.0),
            ("width:400px;max-width:fit-content", 90.0, 10.0),
            ("width:400px;max-width:fit-content(60px)", 60.0, 20.0),
        ] {
            assert_eq!(
                intrinsic_container_size(display, css),
                (width, height),
                "display:{display};{css}"
            );
        }
    }
}

#[test]
fn flex_and_grid_containers_honor_intrinsic_min_max_height_values() {
    for display in ["flex", "grid"] {
        for keyword in [
            "min-content",
            "max-content",
            "fit-content",
            "fit-content(5px)",
        ] {
            assert_eq!(
                intrinsic_container_size(
                    display,
                    &format!("width:40px;height:0;min-height:{keyword}"),
                ),
                (40.0, 20.0),
                "display:{display};min-height:{keyword}"
            );
            assert_eq!(
                intrinsic_container_size(
                    display,
                    &format!("width:40px;height:100px;max-height:{keyword}"),
                ),
                (40.0, 20.0),
                "display:{display};max-height:{keyword}"
            );
        }
    }
}

#[test]
fn flex_grid_items_keep_authored_width_when_measuring_intrinsic_height() {
    let flex_item = flex_item_size(
        "row",
        "width:200px;height:100px",
        "display:grid;width:40px;height:0;min-height:max-content;\
         grid-template-columns:auto;grid-template-rows:auto;\
         justify-items:start;align-items:start",
    );
    assert_eq!(flex_item, (40.0, 20.0), "grid item inside flex");

    let grid_item = grid_item_size(
        "width:200px;grid-template-columns:200px",
        "display:flex;width:40px;height:0;min-height:max-content;align-items:flex-start",
    );
    assert_eq!(grid_item, (40.0, 20.0), "flex item inside grid");
}

#[test]
fn flex_and_grid_min_width_probes_ignore_own_cross_axis_dimensions_and_limits() {
    let mut widths = Vec::new();
    for display in ["flex", "grid"] {
        let (container, item) = match display {
            "flex" => (
                "display:flex;flex-direction:column;align-items:flex-start",
                "flex:1 1 0;height:100%",
            ),
            "grid" => (
                "display:grid;grid-template-columns:auto;grid-template-rows:100%;justify-items:start;align-items:start",
                "height:100%",
            ),
            _ => unreachable!(),
        };
        let css = format!(
            "{container};width:0;min-width:max-content;height:100px;\
             min-height:300px;max-height:400px"
        );
        let mut child = 0;
        let fixture = block_fixture_with_ahem(&css, |doc, root| {
            child =
                doc.append_element(Some(root), "div", crate::LayoutStyle::default(), Some(item));
        });
        let root = fixture.root;
        let mut doc = fixture.doc;
        crate::layout::apply_computed_to_style(&mut doc, &fixture.cascade).expect("bridge");
        doc.layout_node_with_style_mut(child)
            .unwrap()
            .style
            .aspect_ratio = Some(2.0);
        let mut tree = LayoutSession::new(&mut doc);
        let inputs = LayoutInput {
            run_mode: RunMode::ComputeSize,
            sizing_mode: SizingMode::InherentSize,
            axis: RequestedAxis::Both,
            known_dimensions: Size::NONE,
            known_dimensions_are_definite: Size {
                width: false,
                height: false,
            },
            parent_size: Size {
                width: Some(800.0),
                height: Some(600.0),
            },
            available_space: Size {
                width: AvailableSpace::Definite(800.0),
                height: AvailableSpace::MaxContent,
            },
            vertical_margins_are_collapsible: Line::FALSE,
        };
        let output = super::with_content_limits(
            &mut tree,
            NodeId::from(root),
            inputs,
            |tree, node_id, inputs| tree.compute_child_layout_with_block_ctx(node_id, inputs, None),
        );
        widths.push((display, output.size.width));
    }
    assert_eq!(
        widths,
        [("flex", 0.0), ("grid", 0.0)],
        "cross-axis size and limits must not affect intrinsic width"
    );
}

#[test]
fn an_empty_grid_lays_out_fixed_tracks_and_honors_intrinsic_height_limits() {
    for (css, expected_height) in [
        ("height:auto", 100.0),
        ("height:0;min-height:max-content", 100.0),
        ("height:200px;max-height:max-content", 100.0),
    ] {
        let fixture = block_fixture_with_ahem(
            &format!("display:grid;width:100px;grid-template-rows:100px;{css}"),
            |_, _| {},
        );
        let root = fixture.root;
        let mut doc = fixture.doc;
        layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600())
            .expect("layout");
        assert_eq!(absolute_rect(&doc, root).3, expected_height, "{css}");
    }
}

#[test]
fn intrinsic_container_minimum_wins_over_its_maximum() {
    for display in ["flex", "grid"] {
        assert_eq!(
            intrinsic_container_size(
                display,
                "width:20px;min-width:max-content;max-width:min-content",
            ),
            (90.0, 10.0),
            "display:{display}"
        );
    }
}

#[test]
fn nested_flex_and_grid_containers_resolve_their_own_intrinsic_limits() {
    let (mut flex, mut grid) = (0, 0);
    let fixture = block_fixture_with_ahem("width:200px;height:200px", |doc, root| {
        flex = doc.append_element(
            Some(root),
            "div",
            crate::LayoutStyle::default(),
            Some("display:flex;width:20px;min-width:max-content;align-items:flex-start"),
        );
        grid = doc.append_element(
            Some(flex),
            "div",
            crate::LayoutStyle::default(),
            Some(
                "display:grid;width:20px;min-width:max-content;\
                 grid-template-columns:auto;grid-template-rows:auto;\
                 justify-items:start;align-items:start",
            ),
        );
        let item = doc.append_element(
            Some(grid),
            "div",
            crate::LayoutStyle::default(),
            Some("display:block;grid-column:1;grid-row:1"),
        );
        doc.append_text(item, "AAAA BBBB");
    });
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    assert_eq!(absolute_rect(&doc, flex).2, 90.0);
    assert_eq!(absolute_rect(&doc, grid).2, 90.0);
}

#[test]
fn a_flex_container_remeasures_percentage_fit_content_after_available_width_changes() {
    let fixture = block_fixture_with_ahem(
        "display:flex;width:10px;min-width:fit-content(50%);align-items:flex-start",
        |doc, root| {
            let item = doc.append_element(
                Some(root),
                "div",
                crate::LayoutStyle::default(),
                Some("display:block;flex:1 1 auto"),
            );
            doc.append_text(item, "AAAA BBBB");
        },
    );
    let root = fixture.root;
    let mut doc = fixture.doc;
    let mut session = LayoutSession::new(&mut doc);
    let inputs = |width| LayoutInput {
        run_mode: RunMode::PerformLayout,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size::NONE,
        known_dimensions_are_definite: Size {
            width: false,
            height: false,
        },
        parent_size: Size {
            width: Some(width),
            height: None,
        },
        available_space: Size {
            width: AvailableSpace::Definite(width),
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    };
    assert_eq!(
        session
            .compute_child_layout_with_block_ctx(NodeId::from(root), inputs(120.0), None)
            .size
            .width,
        60.0
    );
    assert_eq!(
        session
            .compute_child_layout_with_block_ctx(NodeId::from(root), inputs(80.0), None)
            .size
            .width,
        40.0
    );
}
