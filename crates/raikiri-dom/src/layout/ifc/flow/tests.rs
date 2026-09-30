use super::*;
use crate::layout::ifc::projection::project_ifc;
use crate::layout::ifc::test_support::{ahem_fonts, block_fixture};
use shodo::limits::Limits;

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
