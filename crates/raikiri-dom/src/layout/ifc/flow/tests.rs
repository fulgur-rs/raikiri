use super::*;
use crate::layout::ifc::projection::project_ifc;
use crate::layout::ifc::test_support::{ahem_fonts, block_fixture};
use shodo::limits::Limits;
use taffy::{BlockFormattingContext, Clear, FloatDirection, Size};

fn root_of(css: &str, text: &str) -> (IfcRoot, LayoutContext) {
    // Ahem at 10px: an explicit line-height keeps these tests about breaking,
    // not about how `normal` resolves.
    let css = format!("line-height:10px;{css}");
    let fixture = block_fixture(&css, |doc, root| {
        doc.append_text(root, text);
    });
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
    (IfcRoot::new(projected), cx)
}

#[test]
fn indent_resolves_against_the_content_width() {
    use raikiri_style::ComputedTextIndent as I;
    use raikiri_style::property::CalcLengthPercentage;
    assert_eq!(resolve_indent(I::Px(7.0), 200.0), 7.0);
    assert_eq!(resolve_indent(I::Percent(10.0), 200.0), 20.0);
    assert_eq!(
        resolve_indent(
            I::Calc(CalcLengthPercentage {
                percent: 10.0,
                px: 3.0
            }),
            200.0
        ),
        23.0
    );
}

#[test]
fn break_lines_sums_the_line_advances() {
    let (root, mut cx) = root_of("", "aaaa bbbb cccc");
    // Ahem at 10px: every character is 10px wide and the line is 10px tall.
    let lines = break_lines(&root, &mut cx, 50.0);
    assert_eq!(lines.lines.len(), 3);
    assert_eq!(lines.height, 30.0);
    assert_eq!(lines.width, 50.0);
}

#[test]
fn intrinsic_widths_are_the_longest_word_and_the_whole_line() {
    let (root, mut cx) = root_of("", "aaaa bbbbbb cc");
    assert_eq!(intrinsic_widths(&root, &mut cx), (60.0, 140.0));
}

#[test]
fn text_indent_moves_the_first_line_break() {
    let (root, mut cx) = root_of("text-indent:20px", "aaaa bbbb");
    // 50 - 20 leaves 30px for the first line, so "aaaa" no longer fits.
    let lines = break_lines(&root, &mut cx, 50.0);
    assert_eq!(lines.lines.len(), 2);
}

#[test]
fn first_baseline_is_the_alphabetic_baseline_of_line_one() {
    let (root, mut cx) = root_of("", "aa");
    let lines = break_lines(&root, &mut cx, 100.0);
    // Ahem ascent is 0.8em.
    assert_eq!(first_baseline(&lines), Some(8.0));
}

#[test]
fn last_baseline_adds_the_offset_of_the_last_line() {
    let (root, mut cx) = root_of("", "aaaa bbbb");
    let lines = break_lines(&root, &mut cx, 50.0);
    assert_eq!(lines.lines.len(), 2);
    // `Line::baseline` is relative to its own line; the second line starts
    // 10px below the first.
    assert_eq!(last_baseline(&lines), Some(18.0));
}

/// Text, size and block offset of every line.
fn shape_of(lines: &[shodo::Line]) -> Vec<(String, f32, f32, f32)> {
    lines
        .iter()
        .map(|line| {
            (
                line.text()[line.text_range()].trim_end().to_owned(),
                line.inline_size(),
                line.block_size(),
                line.block_offset(),
            )
        })
        .collect()
}

#[test]
fn place_lines_matches_break_all_without_floats() {
    for (text, width) in [
        ("aaaa bbbb cccc", 50.0_f32),
        ("aaaa bbbb cccc", 100.0),
        ("aaaa bbbb cccc", 500.0),
        ("aa bb cc dd ee ff", 35.0),
    ] {
        let (root, mut cx) = root_of("", text);
        let mut options = root.options;
        options.text_indent.length = resolve_indent(root.indent, width);
        let expected = root
            .paragraph
            .break_all(&mut cx, &options, width, &AtomicSizes::EMPTY);
        let placed = place_lines(
            &root.paragraph,
            &options,
            &mut cx,
            &AtomicSizes::EMPTY,
            |_, _| LineSpace { start: 0.0, width },
        );
        assert_eq!(
            shape_of(&placed.lines),
            shape_of(&expected),
            "{text} @ {width}"
        );
        assert_eq!(placed.width, width);
        assert_eq!(
            placed.height,
            expected.iter().map(|l| l.block_size()).sum::<f32>()
        );
    }
}

#[test]
fn place_lines_asks_for_the_space_at_each_line_offset() {
    let (root, mut cx) = root_of("", "aaaa bbbb cccc");
    let mut asked = Vec::new();
    let placed = place_lines(
        &root.paragraph,
        &root.options,
        &mut cx,
        &AtomicSizes::EMPTY,
        |y, height| {
            asked.push(y);
            let _ = height;
            LineSpace {
                start: 0.0,
                width: 50.0,
            }
        },
    );
    assert_eq!(placed.lines.len(), 3);
    // Every line is laid out at its own block offset (0, 10, 20), and the
    // space is asked again for the height the line turned out to have.
    for offset in [0.0, 10.0, 20.0] {
        assert!(asked.contains(&offset), "{asked:?}");
    }
}

/// A 100px wide context with a left float (30x10 at the top) and a right float
/// (20x30 starting at y=10).
fn two_floats() -> BlockFormattingContext {
    let mut bfc = BlockFormattingContext::new();
    {
        let mut ctx = bfc.root_block_context();
        ctx.set_width(100.0);
        ctx.place_floated_box(
            Size {
                width: 30.0,
                height: 10.0,
            },
            0.0,
            FloatDirection::Left,
            Clear::None,
            false,
        );
        ctx.place_floated_box(
            Size {
                width: 20.0,
                height: 30.0,
            },
            10.0,
            FloatDirection::Right,
            Clear::None,
            false,
        );
    }
    bfc
}

#[test]
fn line_space_reads_the_segment_at_a_line() {
    let mut bfc = two_floats();
    let ctx = bfc.root_block_context();
    // Only the left float is active at y=0..5.
    assert_eq!(
        line_space(&ctx, 0.0, 0.0, 100.0, 0.0, 5.0),
        LineSpace {
            start: 30.0,
            width: 70.0
        }
    );
    // Below both floats the line has the whole width.
    assert_eq!(
        line_space(&ctx, 0.0, 0.0, 100.0, 50.0, 10.0),
        LineSpace {
            start: 0.0,
            width: 100.0
        }
    );
}

#[test]
fn line_space_intersects_the_segments_a_line_spans() {
    let mut bfc = two_floats();
    let ctx = bfc.root_block_context();
    // A 10px line at y=5 spans the left float (until y=10) and the right float
    // (from y=10): it is narrowed by both.
    assert_eq!(
        line_space(&ctx, 0.0, 0.0, 100.0, 5.0, 10.0),
        LineSpace {
            start: 30.0,
            width: 50.0
        }
    );
    // At y=20 only the right float remains.
    assert_eq!(
        line_space(&ctx, 0.0, 0.0, 100.0, 20.0, 10.0),
        LineSpace {
            start: 0.0,
            width: 80.0
        }
    );
}

#[test]
fn line_space_is_relative_to_the_content_box_and_never_wider_than_it() {
    let mut bfc = two_floats();
    let ctx = bfc.root_block_context();
    // A content box that starts 10px in and is 60px wide: the left float still
    // covers 30px of the context, so 20px of this box; the right float does not
    // reach it.
    assert_eq!(
        line_space(&ctx, 10.0, 0.0, 60.0, 0.0, 5.0),
        LineSpace {
            start: 20.0,
            width: 40.0
        }
    );
}
