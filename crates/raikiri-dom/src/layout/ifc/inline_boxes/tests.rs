use super::*;
use crate::layout::ifc::flow::break_lines;
use crate::layout::ifc::projection::project_ifc;
use crate::layout::ifc::root::IfcRoot;
use crate::layout::ifc::test_support::{ahem_fonts, block_fixture, span};
use shodo::geometry::{Direction, PhysicalSize, WritingMode};
use shodo::limits::Limits;
use shodo::{Fragment, LayoutContext};

const EDGES: &str = "padding:0 3px;border-width:0 2px;border-style:solid;margin:0 4px";

/// Break the paragraph `build` fills under a root styled `css` at `width`.
fn lines_of(
    css: &str,
    width: f32,
    build: impl FnOnce(&mut crate::Document, usize),
) -> Vec<shodo::Line> {
    root_and_lines_of(css, width, build).1
}

/// [`lines_of`] with the projected root.
fn root_and_lines_of(
    css: &str,
    width: f32,
    build: impl FnOnce(&mut crate::Document, usize),
) -> (IfcRoot, Vec<shodo::Line>) {
    let css = format!("line-height:10px;{css}");
    let fixture = block_fixture(&css, build);
    let mut cx = LayoutContext::new();
    let projected = project_ifc(
        &fixture.doc,
        &fixture.cascade,
        fixture.root,
        &mut cx,
        &ahem_fonts(),
        &Limits::default(),
    )
    .expect("project");
    let root = IfcRoot::new(projected);
    let lines = break_lines(&root, &mut cx, width).lines.to_vec();
    (root, lines)
}

fn pieces_of(lines: &[shodo::Line], width: f32, rtl: bool) -> Vec<InlineBoxPiece> {
    assert!(
        lines
            .iter()
            .all(|line| { (line.used_direction() == Direction::Rtl) == rtl })
    );
    inline_box_pieces(
        lines,
        WritingMode::HorizontalTb,
        PhysicalSize { width, height: 0.0 },
    )
}

#[test]
fn a_single_line_span_has_the_hand_computed_border_and_content_boxes() {
    let mut span_id = 0;
    let lines = lines_of("width:200px", 200.0, |doc, root| {
        doc.append_text(root, "aa");
        let inner = span(doc, root, &format!("display:inline;{EDGES}"));
        doc.append_text(inner, "bb");
        doc.append_text(root, "cc");
        span_id = inner;
    });
    let pieces = pieces_of(&lines, 200.0, false);
    assert_eq!(pieces.len(), 1);
    let piece = pieces[0];
    assert_eq!(piece.node, span_id);
    assert_eq!(piece.line, 0);
    assert_eq!(
        piece.border_box,
        BoxRect {
            x: 24.0,
            y: 0.0,
            width: 30.0,
            height: 10.0
        }
    );
    assert_eq!(
        piece.content_box,
        BoxRect {
            x: 29.0,
            y: 0.0,
            width: 20.0,
            height: 10.0
        }
    );
    assert!(piece.has_start_edge && piece.has_end_edge);
    assert_eq!(piece.parent, None);
}

#[test]
fn vertical_padding_and_border_grow_the_box_but_not_the_line() {
    let lines = lines_of("width:200px", 200.0, |doc, root| {
        let inner = span(
            doc,
            root,
            "display:inline;padding:1px 0;border-width:2px 0;border-style:solid",
        );
        doc.append_text(inner, "bb");
    });
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].block_size(), 10.0);
    let piece = pieces_of(&lines, 200.0, false)[0];
    // above = below = padding 1 + border 2.
    assert_eq!(
        piece.border_box,
        BoxRect {
            x: 0.0,
            y: -3.0,
            width: 20.0,
            height: 16.0
        }
    );
    assert_eq!(
        piece.content_box,
        BoxRect {
            x: 0.0,
            y: 0.0,
            width: 20.0,
            height: 10.0
        }
    );
}

#[test]
fn vertical_inline_pieces_use_physical_coordinates_for_each_writing_mode() {
    for (mode, shodo_mode, expected_x) in [
        ("vertical-rl", WritingMode::VerticalRl, 93.0),
        ("vertical-lr", WritingMode::VerticalLr, -3.0),
    ] {
        let css = format!("width:100px;height:40px;writing-mode:{mode}");
        let (root, lines) = root_and_lines_of(&css, 40.0, |doc, root| {
            let inline = span(
                doc,
                root,
                "display:inline;padding-left:3px;padding-right:3px",
            );
            doc.append_text(inline, "bb");
        });
        assert_eq!(root.writing_mode, shodo_mode);
        assert_eq!(lines.len(), 1);

        let piece = inline_box_pieces(
            &lines,
            root.writing_mode,
            PhysicalSize {
                width: 100.0,
                height: 40.0,
            },
        )[0];
        assert_eq!(
            (piece.border_box.x, piece.border_box.y),
            (expected_x, 0.0),
            "{mode}"
        );
        assert_eq!(
            (piece.border_box.width, piece.border_box.height),
            (10.0, 26.0),
            "{mode}"
        );
    }
}

#[test]
fn an_rtl_inline_span_does_not_reverse_vertical_block_progression() {
    let (root, lines) = root_and_lines_of(
        "width:100px;height:40px;writing-mode:vertical-rl;direction:ltr",
        40.0,
        |doc, root| {
            let inline = span(doc, root, "display:inline;direction:rtl");
            doc.append_text(inline, "bb");
        },
    );
    assert_eq!(lines[0].used_direction(), Direction::Ltr);

    let piece = inline_box_pieces(
        &lines,
        root.writing_mode,
        PhysicalSize {
            width: 100.0,
            height: 40.0,
        },
    )[0];
    assert_eq!(piece.border_box.x, 93.0);
}

#[test]
fn a_span_that_wraps_has_a_piece_per_line_and_only_the_ends_keep_their_edges() {
    // "aaaa bbbb" does not fit in 60px: the span wraps after "aaaa ".
    let lines = lines_of("width:60px", 60.0, |doc, root| {
        let inner = span(doc, root, &format!("display:inline;{EDGES}"));
        doc.append_text(inner, "aaaa bbbb");
    });
    assert_eq!(lines.len(), 2);
    let pieces = pieces_of(&lines, 60.0, false);
    assert_eq!(pieces.len(), 2);
    assert_eq!((pieces[0].line, pieces[1].line), (0, 1));
    assert!(pieces[0].has_start_edge && !pieces[0].has_end_edge);
    assert!(!pieces[1].has_start_edge && pieces[1].has_end_edge);
    // Line 1: margin 4, border 2 + padding 3, "aaaa" 40; the hung space is
    // not part of the piece. Line 2: "bbbb" 40, padding 3 + border 2.
    assert_eq!(
        pieces[0].border_box,
        BoxRect {
            x: 4.0,
            y: 0.0,
            width: 45.0,
            height: 10.0
        }
    );
    assert_eq!(
        pieces[1].border_box,
        BoxRect {
            x: 0.0,
            y: 10.0,
            width: 45.0,
            height: 10.0
        }
    );
}

#[test]
fn the_hung_trailing_space_is_not_part_of_a_continuing_piece() {
    // The line "aaaa " keeps the collapsible space hanging at its end.
    let lines = lines_of("width:60px", 60.0, |doc, root| {
        let inner = span(doc, root, "display:inline");
        doc.append_text(inner, "aaaa bbbb");
    });
    let first = pieces_of(&lines, 60.0, false)[0];
    assert_eq!(first.border_box.width, 40.0, "the 10px space is hanging");
    assert_eq!(first.content_box.width, 40.0);
    assert!(lines[0].hang_end() > 0.0);
}

#[test]
fn a_span_that_closes_before_the_hung_space_keeps_its_width() {
    // The space that hangs at the end of line 1 follows the span, so the
    // span's own piece is not shortened by it.
    let lines = lines_of("width:60px", 60.0, |doc, root| {
        let inner = span(doc, root, &format!("display:inline;{EDGES}"));
        doc.append_text(inner, "aaaa");
        doc.append_text(root, " bbbb");
    });
    assert_eq!(lines.len(), 2);
    let pieces = pieces_of(&lines, 60.0, false);
    assert_eq!(pieces.len(), 1);
    // margin 4, border 2 + padding 3, "aaaa" 40, padding 3 + border 2.
    assert_eq!(
        pieces[0].border_box,
        BoxRect {
            x: 4.0,
            y: 0.0,
            width: 50.0,
            height: 10.0
        }
    );
}

#[test]
fn nested_spans_record_their_parent() {
    let (mut outer_id, mut inner_id) = (0, 0);
    let lines = lines_of("width:200px", 200.0, |doc, root| {
        doc.append_text(root, "a");
        let outer = span(doc, root, "display:inline;padding:0 1px");
        doc.append_text(outer, "b");
        let inner = span(doc, outer, "display:inline;padding:0 2px");
        doc.append_text(inner, "c");
        outer_id = outer;
        inner_id = inner;
    });
    let pieces = pieces_of(&lines, 200.0, false);
    let inner = pieces.iter().find(|p| p.node == inner_id).expect("inner");
    let outer = pieces.iter().find(|p| p.node == outer_id).expect("outer");
    assert_eq!(inner.parent, Some(outer_id));
    assert_eq!(outer.parent, None);
    assert_eq!(
        outer.border_box,
        BoxRect {
            x: 10.0,
            y: 0.0,
            width: 26.0,
            height: 10.0
        }
    );
    assert_eq!(
        inner.border_box,
        BoxRect {
            x: 21.0,
            y: 0.0,
            width: 14.0,
            height: 10.0
        }
    );
}

#[test]
fn right_to_left_pieces_are_mirrored_inside_the_content_width() {
    let lines = lines_of("width:100px;direction:rtl", 100.0, |doc, root| {
        let inner = span(doc, root, "display:inline;padding:0 3px");
        doc.append_text(inner, "bb");
    });
    // Logical inline_start 0, size 26: mirrored x = 100 - (0 + 26) = 74.
    let piece = pieces_of(&lines, 100.0, true)[0];
    assert_eq!(piece.border_box.x, 74.0);
    assert_eq!(piece.border_box.width, 26.0);
}

#[test]
fn a_span_that_closes_after_the_hung_space_ends_before_it() {
    // The span's own last character is the space that hangs at the end of
    // line 1; CSS Text 3 4.1.3 removes it, so the closing padding and border
    // follow "aaaa" directly.
    let lines = lines_of("width:60px", 60.0, |doc, root| {
        let inner = span(doc, root, &format!("display:inline;{EDGES}"));
        doc.append_text(inner, "aaaa ");
        doc.append_text(root, "bbbb");
    });
    assert_eq!(lines.len(), 2);
    assert!(lines[0].hang_end() > 0.0);
    let piece = pieces_of(&lines, 60.0, false)[0];
    assert!(piece.has_end_edge);
    // margin 4, border 2 + padding 3, "aaaa" 40, padding 3 + border 2.
    assert_eq!(
        piece.border_box,
        BoxRect {
            x: 4.0,
            y: 0.0,
            width: 50.0,
            height: 10.0
        }
    );
    assert_eq!(piece.content_box.width, 40.0);
}

#[test]
fn an_element_around_a_closed_span_loses_the_hung_space_too() {
    // The outer span continues on line 2; the inner one closes after the
    // hung space. Neither keeps the space.
    let (mut outer_id, mut inner_id) = (0, 0);
    let lines = lines_of("width:60px", 60.0, |doc, root| {
        let outer = span(doc, root, "display:inline");
        let inner = span(doc, outer, "display:inline;padding:0 3px");
        doc.append_text(inner, "aaaa ");
        doc.append_text(outer, "bbbb");
        outer_id = outer;
        inner_id = inner;
    });
    assert_eq!(lines.len(), 2);
    let pieces = pieces_of(&lines, 60.0, false);
    let on_line = |node: usize| {
        pieces
            .iter()
            .find(|p| p.node == node && p.line == 0)
            .expect("piece on line 1")
            .border_box
    };
    // Inner: padding 3, "aaaa" 40, padding 3 = 46; outer ends with it.
    assert_eq!(on_line(inner_id).width, 46.0);
    assert_eq!(on_line(outer_id).width, 46.0);
}

#[test]
fn a_right_to_left_wrapped_piece_keeps_its_width() {
    // In a right-to-left line the hung space sits after the span's piece, at
    // the line's left end, so the piece has nothing to give up.
    let lines = lines_of("width:60px;direction:rtl", 60.0, |doc, root| {
        let inner = span(doc, root, &format!("display:inline;{EDGES}"));
        doc.append_text(inner, "aaaa bbbb");
    });
    assert_eq!(lines.len(), 2);
    let pieces = pieces_of(&lines, 60.0, true);
    // Line 1: margin 4 from the right edge, border box 45 (border 2 +
    // padding 3 + "aaaa"): x 11..56. Line 2: "bbbb" and the end edges, 45
    // wide from the right edge: x 15..60.
    assert_eq!(
        pieces[0].border_box,
        BoxRect {
            x: 11.0,
            y: 0.0,
            width: 45.0,
            height: 10.0
        }
    );
    assert_eq!(
        pieces[1].border_box,
        BoxRect {
            x: 15.0,
            y: 10.0,
            width: 45.0,
            height: 10.0
        }
    );
}

#[test]
fn preserved_spaces_that_hang_stay_in_their_element() {
    // With preserved spaces the three spaces after "aaaa" are kept and hang
    // past the 60px line (CSS Text 3 4.1.3: preserved spaces hang, they are
    // not removed); the span holding them keeps its 30px box (x 40..70) as
    // ink overflow.
    for css in [
        "width:60px;white-space:pre-wrap",
        "width:60px;white-space-collapse:preserve-spaces",
    ] {
        let mut span_id = 0;
        let (_, lines) = root_and_lines_of(css, 60.0, |doc, root| {
            doc.append_text(root, "aaaa");
            let inner = span(doc, root, "display:inline");
            doc.append_text(inner, "   ");
            doc.append_text(root, "bb");
            span_id = inner;
        });
        assert_eq!(lines.len(), 2, "{css}");
        assert!(lines[0].hang_end() > 0.0, "{css}");
        let pieces = pieces_of(&lines, 60.0, false);
        let piece = pieces.iter().find(|p| p.node == span_id).expect("span");
        assert_eq!(
            piece.border_box,
            BoxRect {
                x: 40.0,
                y: 0.0,
                width: 30.0,
                height: 10.0
            },
            "{css}"
        );
    }
}

#[test]
fn inline_box_geometry_matches_shodo_when_spaces_hang() {
    for (css, rtl, preserved) in [
        ("width:60px", false, false),
        ("width:60px;direction:rtl", true, false),
        ("width:60px;white-space:pre-wrap", false, true),
    ] {
        let mut span_id = 0;
        let (_, lines) = root_and_lines_of(css, 60.0, |doc, root| {
            let inner = span(doc, root, "display:inline");
            if preserved {
                doc.append_text(root, "aaaa");
                doc.append_text(inner, "   ");
                doc.append_text(root, "bb");
            } else {
                doc.append_text(inner, "aaaa ");
                doc.append_text(root, "bbbb");
            }
            span_id = inner;
        });
        assert!(lines[0].hang_end() > 0.0, "{css}");

        let raw = lines[0]
            .fragments()
            .find_map(|fragment| match fragment {
                Fragment::InlineBox(fragment) if fragment.node.0 as usize == span_id => {
                    Some(fragment)
                }
                _ => None,
            })
            .expect("shodo inline-box fragment");
        let piece = pieces_of(&lines, 60.0, rtl)
            .into_iter()
            .find(|piece| piece.node == span_id && piece.line == 0)
            .expect("Raikiri inline-box piece");
        let expected_x = if rtl {
            60.0 - (raw.rect.inline_start + raw.rect.inline_size)
        } else {
            raw.rect.inline_start
        };
        assert_eq!(piece.border_box.x, expected_x, "{css}");
        assert_eq!(piece.border_box.width, raw.rect.inline_size, "{css}");
        assert_eq!(
            piece.content_box.width, raw.content_rect.inline_size,
            "{css}"
        );
    }
}
