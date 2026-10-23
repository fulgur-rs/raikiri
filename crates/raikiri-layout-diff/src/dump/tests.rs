use super::*;
use crate::tests::text_of;

/// A document that reaches the generated, line-piece and replaced boxes, a
/// page-margin box, a link, text decorations and shadows, and several pages.
const RICH: &str = "<!doctype html><html><head><style>
@page { size: 200px 100px; margin: 20px; @top-center { content: 'Header ' counter(page) } }
body { margin: 0; font: 10px/12px Ahem }
p::before { content: 'x'; border: 1px solid; background: #eee; padding: 1px }
.first-line::first-line { color: rgba(0, 0, 0, .5) }
.first-line span { border: 1px solid; background: yellow }
.under { text-decoration: underline; text-shadow: 1px 1px 2px red }
</style></head><body>
<p>hello <a href='#end'>link</a></p>
<p class='first-line'><span>aa bb cc dd ee ff gg hh ii jj kk ll mm nn oo pp qq</span></p>
<p class='under'>under</p>
<img width=10 height=10 alt=x>
<div style='height:150px'></div><p id='end'>end</p>
</body></html>";

/// A document with separated-border table rows that carry shadows, a column
/// rule, and a first page of another width than the rest.
const TABLES_AND_COLUMNS: &str = "<!doctype html><html><head><style>
@page { size: 200px 100px; margin: 10px }
@page :first { size: 300px 100px; margin: 10px }
body { margin: 0; font: 10px/12px Ahem }
table { border-collapse: separate; border-spacing: 4px }
tr { box-shadow: 2px 2px 2px #000; background: #ccc }
td { border: 1px solid }
.columns { columns: 2; column-rule: 1px solid; height: 40px }
</style></head><body>
<table><tr><td>cell</td><td>b</td></tr><tr><td>row2</td><td>x</td></tr></table>
<div class='columns'>aaa bbb ccc ddd eee fff ggg hhh iii jjj</div>
<div style='height:200px'></div><p>end</p>
</body></html>";

#[test]
fn fnv1a_hashes_known_text() {
    assert_eq!(fnv1a(""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(fnv1a("a"), 0xaf63_dc4c_8601_ec8c);
    assert_eq!(fnv1a("foobar"), 0x85944171f73967e8);
    assert_ne!(fnv1a("ab"), fnv1a("ba"));
}

#[test]
fn numbers_are_equal_exactly_when_their_bits_are() {
    let one_ulp_up = f32::from_bits(1.0f32.to_bits() + 1);
    let values = [
        0.0,
        -0.0,
        1.0,
        one_ulp_up,
        100.0,
        100.001,
        f32::MIN_POSITIVE,
        f32::MAX,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NAN,
        f32::from_bits(0x7fc0_0001),
        f32::from_bits(0xffc0_0000),
    ];
    for a in values {
        for b in values {
            assert_eq!(
                number(a) == number(b),
                a.to_bits() == b.to_bits(),
                "{:08x} {:08x}",
                a.to_bits(),
                b.to_bits()
            );
        }
    }
    assert_eq!(number(10.5), "10.5");
    assert_eq!(number(-0.0), "-0.0");
    assert_eq!(number(f32::NAN), "NaN:7fc00000");
    assert_eq!(number(f32::INFINITY), "inf");
}

#[test]
fn lists_and_rectangles_read_in_a_fixed_order() {
    assert_eq!(numbers(&[1.0, -2.5]), "[1.0 -2.5]");
    assert_eq!(numbers(&[]), "[]");
    assert_eq!(
        rect(&PaintRect::new(1.0, 2.0, 3.0, 4.5)),
        "[1.0 2.0 3.0 4.5]"
    );
}

#[test]
fn a_thousandth_of_a_pixel_shows_in_the_text() {
    let page = |width: &str| {
        text_of(&format!(
            "<div style='width:{width}; height:20px'><p>hello</p></div>"
        ))
    };
    let base = page("100px");
    assert_eq!(base, page("100px"));
    let moved = page("100.001px");
    assert_ne!(base, moved);
    // The two layouts have the same shape; only numbers differ.
    assert_eq!(base.lines().count(), moved.lines().count());
    assert!(base.lines().zip(moved.lines()).any(|(a, b)| a != b));
}

#[test]
fn the_page_api_is_written_for_every_page() {
    let text = text_of(RICH);
    assert!(
        text.starts_with("pages: 5\nwarnings: 0\npage 0 name=None\n"),
        "{text}"
    );
    for expected in [
        "geometry: PageGeometry {",
        "root_box: Some(RootBox {",
        "fragments: ",
        "  fragment node=",
        "overflow_clips: 0",
        "text_runs: ",
        "  run line=",
        "    glyph id=",
        "    decoration DecorationLine {",
        "    shadow TextShadow {",
        "paint_order: ",
        "  box node=",
        "  text node=",
        "  replaced node=",
        "  generated_box owner=",
        "  line_box node=",
        "    line=TextLineId {",
        "margin_boxes: 1\nMarginBox {",
        "links: 1\nLink {",
    ] {
        assert!(text.contains(expected), "{expected:?} is missing:\n{text}");
    }
    assert_eq!(text.matches("\npage ").count(), 5, "every page is listed");
    assert!(text.contains("\ndocument 0 (first used by page 0)\nnodes: "));
    assert_eq!(text.matches("\ndocument ").count(), 1);
}

#[test]
fn shadows_clips_and_column_rules_are_written_as_paint_events() {
    let text = text_of(TABLES_AND_COLUMNS);
    for expected in [
        "  outer_box_shadow node=",
        "  PushClip(",
        "  PopClip",
        "  ColumnRule(",
        "overflow_clips: ",
    ] {
        assert!(text.contains(expected), "{expected:?} is missing:\n{text}");
    }
}

#[test]
fn pages_of_another_width_are_listed_with_their_own_document() {
    let text = text_of(TABLES_AND_COLUMNS);
    assert!(
        text.contains("\ndocument 0 (first used by page 0)\n"),
        "{text}"
    );
    assert!(
        text.contains("\ndocument 1 (first used by page 1)\n"),
        "{text}"
    );
}

#[test]
fn warnings_are_written_before_the_pages() {
    let text = text_of(
        "<style>@font-face { font-family: Missing; src: url(missing.ttf) } \
         p { font-family: Missing }</style><p>x</p>",
    );
    assert!(text.starts_with("pages: 1\nwarnings: "), "{text}");
    assert!(!text.starts_with("pages: 1\nwarnings: 0"), "{text}");
    assert!(text.contains("\n  RenderWarning {"), "{text}");
}

#[test]
fn every_node_gets_a_line_with_its_box() {
    let text =
        text_of("<p style='margin:3px 4px 5px 6px; padding:2px; border:1px solid'>quoted text</p>");
    let line = text
        .lines()
        .find(|line| line.contains("tag=p "))
        .expect("the paragraph has a line");
    assert!(
        line.contains("padding=[2.0 2.0 2.0 2.0]")
            && line.contains("border=[1.0 1.0 1.0 1.0]")
            && line.contains("margin=[6.0 4.0 3.0 5.0]"),
        "{line}"
    );
    assert!(
        line.starts_with('n') && line.contains(" Element "),
        "{line}"
    );
    assert!(
        text.contains(" Text tag=- text=\"quoted text\" parent="),
        "{text}"
    );
    assert!(text.contains("tag=html text=\"\" parent=0 "), "{text}");
}

#[test]
fn long_text_is_quoted_up_to_a_limit() {
    let text = text_of("<p>abcdefghijklmnopqrstuvwxyz0123456789</p>");
    assert!(
        text.contains("text=\"abcdefghijklmnopqrstuvwx…\""),
        "{text}"
    );
    let text = text_of("<p>abcdefghijklmnopqrstuvwx</p>");
    assert!(text.contains("text=\"abcdefghijklmnopqrstuvwx\""), "{text}");
}

#[test]
fn the_text_of_margin_boxes_is_written_glyph_by_glyph() {
    let text = text_of(
        "<style>@page { size: 300px 200px; margin: 40px; \
         @top-center { content: 'Header ' counter(page) } \
         @left-middle { content: 'Side'; writing-mode: vertical-rl } } \
         body { margin: 0; font: 20px/20px Ahem }</style><p>hello</p>",
    );
    assert!(text.contains("margin_boxes: 2\n"), "{text}");
    // A box with horizontal text has runs like the text of the page.
    assert!(text.contains("source=MarginBox(TopCenter)"), "{text}");
    assert!(text.contains("text=\"Header 1\""), "{text}");
    assert!(
        text.contains("  shaped StandaloneText { lines: [Line {"),
        "{text}"
    );
    assert!(text.contains("    line 0 mode=HorizontalTb "), "{text}");
    // A box in a vertical writing mode has none, so its lines list their
    // pieces.
    assert!(!text.contains("source=MarginBox(LeftMiddle)"), "{text}");
    assert!(text.contains("    line 0 mode=VerticalRl "), "{text}");
    assert!(text.contains("      GlyphRun(GlyphRunView {"), "{text}");
    // The pieces of horizontal lines are the runs above, not repeated.
    assert_eq!(text.matches("      GlyphRun(").count(), 1, "{text}");
}

#[test]
fn a_page_that_moves_margin_box_text_changes_the_text() {
    let page = |align: &str| {
        text_of(&format!(
            "<style>@page {{ size: 300px 200px; margin: 40px; \
             @top-center {{ content: 'Header'; text-align: {align} }} }} \
             body {{ margin: 0 }}</style><p>x</p>"
        ))
    };
    assert_eq!(page("left"), page("left"));
    assert_ne!(page("left"), page("right"));
}

#[test]
fn images_of_content_and_markers_are_written_with_their_placement() {
    let text = text_of(
        "<style>@page { size: 200px 100px; margin: 0 } \
         body { margin: 0; font: 20px/20px Ahem } p { margin: 0 } \
         .a::before { content: 'ab ' url(https://example.test/i.png) ' cd' } \
         .b::after { content: url(https://example.test/j.png) } \
         ul { list-style-image: url(https://example.test/k.png) }</style>\
         <p class=a>x</p><p class=b>y</p><ul><li>item</li></ul>",
    );
    for expected in [
        "  marker_image owner=",
        "placement=https://example.test/k.png [",
        "  generated_image owner=",
        " kind=Before index=0 placement=https://example.test/i.png [",
        " kind=After index=0 placement=https://example.test/j.png [",
        " text_position=Some((TextLineId {",
    ] {
        assert!(text.contains(expected), "{expected:?} is missing:\n{text}");
    }
    // Every image is the same placeholder, so a content image is 8 by 4 px.
    assert!(text.contains(" 8.0 4.0]"), "{text}");
}

#[test]
fn an_image_event_without_a_placement_says_so() {
    assert_eq!(placement(None), "-");
}
