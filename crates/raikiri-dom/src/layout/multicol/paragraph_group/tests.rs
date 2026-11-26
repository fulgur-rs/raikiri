//! Unit tests for the paragraph planner's helpers.

use super::*;

/// The padding at the start or the end of a wrapper, `height` tall.
fn edge(height: f32, opens: Option<usize>, closes: bool) -> Option<Paragraph> {
    Some(Paragraph {
        opens,
        closes,
        sliced: true,
        ..Paragraph::whole_box(height, 0.0, 0.0, None)
    })
}

#[test]
fn a_sized_box_with_no_lines_adds_nothing_below_the_padding() {
    let paragraphs = [
        edge(3.0, Some(2), false),
        Some(Paragraph {
            sized_leaf: true,
            ..Paragraph::whole_box(20.0, 0.0, 0.0, None)
        }),
        edge(4.0, None, true),
    ];
    assert_eq!(first_piece(&paragraphs, 3.0, 1, 2), 3.0);
}

#[test]
fn a_wrapper_that_holds_nothing_takes_its_paddings() {
    let paragraphs = [edge(3.0, Some(2), false), None, edge(4.0, None, true)];
    assert_eq!(first_piece(&paragraphs, 3.0, 1, 2), 7.0);
    let empty = || {
        Some(Paragraph {
            extents: Vec::new(),
            ..Paragraph::whole_box(0.0, 0.0, 0.0, None)
        })
    };
    let paragraphs = [edge(3.0, Some(2), false), empty(), empty()];
    assert_eq!(first_piece(&paragraphs, 3.0, 1, 2), 3.0);
}
