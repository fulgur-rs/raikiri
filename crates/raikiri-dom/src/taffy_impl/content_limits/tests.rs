//! `min-width`, `max-width`, `min-height` and `max-height` set to an intrinsic
//! sizing keyword take the size the keyword measures (CSS Sizing 3 §3.1.1, §4
//! and §5). The text of the fixtures is `AAAA BBBB` in Ahem at 10px: 40px wide
//! at its longest word, 90px wide on one line.

use super::{applies, stretch_fit_width};
use crate::layout::context::LayoutSession;
use crate::layout::ifc::test_support::{block_fixture, block_fixture_with_ahem};
use crate::layout::layout_single_page;
use crate::layout::test_support::{absolute_rect, page_box_800x600, with_ahem};
use crate::node::FormattingContext;
use taffy::{AvailableSpace, LayoutInput, Line, NodeId, RequestedAxis, RunMode, Size, SizingMode};

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
            taffy::Style::default(),
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
                    taffy::Style::default(),
                    Some(&format!("display:block;width:{width}px;height:{height}px")),
                );
            }
            Body::InlineBlocks(width) => {
                for _ in 0..2 {
                    doc.append_element(
                        Some(child),
                        "span",
                        taffy::Style::default(),
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
            taffy::Style::default(),
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
            taffy::Style::default(),
            Some("display:block;min-width:fit-content;width:10px"),
        );
        for _ in 0..2 {
            doc.append_element(
                Some(inner),
                "span",
                taffy::Style::default(),
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
            taffy::Style::default(),
            Some("display:block;max-width:fit-content;width:200px"),
        );
        for _ in 0..2 {
            doc.append_element(
                Some(inner),
                "span",
                taffy::Style::default(),
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
fn the_stretch_fit_width_is_the_containing_block_less_margins_within_the_room() {
    let mut child = 0;
    let fixture = block_fixture("width: 200px", |doc, root| {
        child = doc.append_element(
            Some(root),
            "div",
            taffy::Style::default(),
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
                taffy::Style::default(),
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
fn limits_are_resolved_for_block_and_paragraph_boxes_that_are_not_being_measured() {
    let mut child = 0;
    let fixture = block_fixture("width: 200px", |doc, root| {
        child = doc.append_element(
            Some(root),
            "div",
            taffy::Style::default(),
            Some("display:block;min-width:min-content"),
        );
    });
    let mut doc = fixture.doc;
    let mut session = LayoutSession::new(&mut doc);
    let limits = session.nodes[child].content_limits;
    assert_ne!(limits, super::ContentLimits::NONE);
    for context in [FormattingContext::Block, FormattingContext::Inline] {
        session.nodes[child].formatting_context = context;
        assert!(applies(&session, child, &limits));
    }
    // A box laid out by another algorithm keeps the lengths of its style.
    session.nodes[child].formatting_context = FormattingContext::Flex;
    assert!(!applies(&session, child, &limits));
    // So does a box that is being measured without its limits.
    session.nodes[child].formatting_context = FormattingContext::Block;
    session.context.content_limit_probes.push(child);
    assert!(!applies(&session, child, &limits));
    // And a box without any.
    session.context.content_limit_probes.clear();
    assert!(!applies(&session, child, &super::ContentLimits::NONE));
}
