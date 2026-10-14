use super::*;
use crate::layout::PositionedLines;
use crate::layout::ifc::test_support::{Fixture, block_fixture, sheet_fixture};
use crate::layout::layout_single_page;
use crate::layout::test_support::{page_box_800x600, with_ahem};
use taffy::Style;

fn element(doc: &mut Document, parent: usize, tag: &str, style: &str) -> usize {
    doc.append_element(Some(parent), tag, Style::default(), Some(style))
}

/// `<tag style="{style}">{text}</tag>` under `parent`.
fn text_element(doc: &mut Document, parent: usize, tag: &str, style: &str, text: &str) -> usize {
    let id = element(doc, parent, tag, style);
    doc.append_text(id, text);
    id
}

fn ruby(doc: &mut Document, parent: usize) -> usize {
    element(doc, parent, "ruby", "display:ruby")
}

fn rb(doc: &mut Document, parent: usize, text: &str) -> usize {
    text_element(doc, parent, "rb", "display:ruby-base", text)
}

fn rt(doc: &mut Document, parent: usize, text: &str) -> usize {
    text_element(doc, parent, "rt", "display:ruby-text;font-size:5px", text)
}

/// The text of the nodes in `ids`, with white space shown as `_`.
fn text_of(doc: &Document, cascade: &CascadeResult, ids: &[usize]) -> String {
    texts(doc, cascade, ids)
        .into_iter()
        .map(|id| doc.nodes[id].text_content().unwrap_or_default().to_owned())
        .collect::<String>()
        .replace(' ', "_")
}

/// A part as `rb(text)` for an element or `(text)` for an anonymous box,
/// with `+tail` when intra-level white space follows it.
fn describe_part(doc: &Document, cascade: &CascadeResult, part: &Part) -> String {
    let tag = part
        .element
        .and_then(|id| doc.nodes[id].tag_name())
        .unwrap_or_default();
    let tail = if part.tail.is_empty() { "" } else { "+tail" };
    format!("{tag}({}){tail}", text_of(doc, cascade, &part.children))
}

/// One line per segment: `gap | bases | level; level`.
fn describe(doc: &Document, cascade: &CascadeResult, plan: &Plan) -> Vec<String> {
    plan.segments
        .iter()
        .map(|segment| {
            let parts = |parts: &[Part]| {
                parts
                    .iter()
                    .map(|part| describe_part(doc, cascade, part))
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            let levels = segment
                .levels
                .iter()
                .map(|level| {
                    let tag = level
                        .container
                        .and_then(|id| doc.nodes[id].tag_name())
                        .unwrap_or_default();
                    format!("{tag}[{}]", parts(&level.annotations))
                })
                .collect::<Vec<_>>()
                .join("; ");
            format!(
                "{}|{}|{levels}",
                text_of(doc, cascade, &segment.gap),
                parts(&segment.bases)
            )
        })
        .collect()
}

fn plan_of(build: impl FnOnce(&mut Document, usize)) -> Vec<String> {
    let mut container = 0;
    let fixture = block_fixture("", |doc, root| {
        container = ruby(doc, root);
        build(doc, container);
    });
    let plan = plan(&fixture.doc, &fixture.cascade, container).expect("plain ruby");
    describe(&fixture.doc, &fixture.cascade, &plan)
}

#[test]
fn bases_pair_with_the_annotations_after_them() {
    let plan = plan_of(|doc, ruby| {
        doc.append_text(ruby, "漢");
        rt(doc, ruby, "かん");
        doc.append_text(ruby, " ");
        rb(doc, ruby, "字");
        rt(doc, ruby, "じ");
    });
    assert_eq!(plan, ["|(漢)|[rt(かん)]", "_|rb(字)|[rt(じ)]"]);
}

#[test]
fn consecutive_annotations_form_one_level_and_containers_their_own() {
    let plan = plan_of(|doc, ruby| {
        rb(doc, ruby, "a");
        rb(doc, ruby, "b");
        rt(doc, ruby, "x");
        doc.append_text(ruby, " ");
        rt(doc, ruby, "y");
        let rtc = element(doc, ruby, "rtc", "display:ruby-text-container");
        doc.append_text(rtc, "z");
    });
    assert_eq!(plan, ["|rb(a) rb(b)|[rt(x)+tail rt(y)]; rtc[(z)]"]);
}

#[test]
fn white_space_between_bases_is_kept_and_before_annotations_dropped() {
    let plan = plan_of(|doc, ruby| {
        doc.append_text(ruby, " ");
        rb(doc, ruby, "a");
        doc.append_text(ruby, " ");
        rb(doc, ruby, "b");
        doc.append_text(ruby, " ");
        rt(doc, ruby, "x");
        doc.append_text(ruby, " ");
    });
    assert_eq!(plan, ["|rb(a)+tail rb(b)|[rt(x)]"]);
}

#[test]
fn text_containers_split_their_runs_into_annotations() {
    let plan = plan_of(|doc, ruby| {
        rb(doc, ruby, "a");
        rb(doc, ruby, "b");
        let rtc = element(doc, ruby, "rtc", "display:ruby-text-container");
        doc.append_text(rtc, " ");
        rt(doc, rtc, "x");
        doc.append_text(rtc, " ");
        rt(doc, rtc, "y");
        doc.append_text(rtc, "z ");
        let rtc = element(doc, ruby, "rtc", "display:ruby-text-container");
        rt(doc, rtc, "w");
        doc.append_text(rtc, " ");
    });
    assert_eq!(
        plan,
        ["|rb(a) rb(b)|rtc[rt(x)+tail rt(y) (z_)]; rtc[rt(w)]"]
    );
}

#[test]
fn base_containers_add_their_bases() {
    let plan = plan_of(|doc, ruby| {
        let rbc = element(doc, ruby, "rbc", "display:ruby-base-container");
        rb(doc, rbc, "a");
        doc.append_text(rbc, " ");
        rb(doc, rbc, "b");
        doc.append_text(rbc, "c");
        rt(doc, ruby, "x");
        doc.append_text(ruby, "d");
    });
    assert_eq!(plan, ["|rb(a)+tail rb(b) (c)|[rt(x)]", "|(d)|"]);
}

#[test]
fn inline_content_nested_rubies_and_floats_stay_plain() {
    let mut container = 0;
    let mut float = 0;
    let fixture = block_fixture("", |doc, root| {
        container = ruby(doc, root);
        let span = element(doc, container, "span", "display:inline");
        doc.append_text(span, "a");
        float = element(doc, span, "i", "float:left");
        let inner = ruby(doc, container);
        rb(doc, inner, "b");
        rt(doc, inner, "y");
        rt(doc, container, "x");
    });
    let plan = plan(&fixture.doc, &fixture.cascade, container).expect("plain ruby");
    assert_eq!(
        describe(&fixture.doc, &fixture.cascade, &plan),
        ["|(aby)|[rt(x)]"]
    );
    assert_eq!(plan.out_of_flow, [(1, float)]);
}

#[test]
fn display_contents_content_stays_plain() {
    let mut ids = (0, 0);
    let fixture = block_fixture("line-height:20px", |doc, root| {
        let container = ruby(doc, root);
        let contents = element(doc, container, "span", "display:contents");
        ids.0 = doc.append_text(contents, "a");
        let annotation = rt(doc, container, "x");
        ids.1 = doc.nodes[annotation].children[0];
    });
    let container = fixture.doc.nodes[fixture.root].children[0];
    let plan = plan(&fixture.doc, &fixture.cascade, container).expect("plain ruby");
    assert_eq!(
        describe(&fixture.doc, &fixture.cascade, &plan),
        ["|(a)|[rt(x)]"]
    );
    let runs = runs(fixture);
    let base = runs.iter().find(|run| run.0 == ids.0).expect("base");
    let annotation = runs.iter().find(|run| run.0 == ids.1).expect("annotation");
    assert!(annotation.1 && !base.1);
    assert!(annotation.2.1 < base.2.1, "{runs:?}");
}

#[test]
fn inline_boxes_in_annotations_have_pieces_over_their_bases() {
    let mut ids = (0, 0);
    let fixture = block_fixture("line-height:20px", |doc, root| {
        let outer = ruby(doc, root);
        let inner = ruby(doc, outer);
        ids.0 = text_element(doc, inner, "rb", "display:ruby-base", "a");
        let nested = element(doc, inner, "rt", "display:ruby-text;font-size:5px");
        text_element(doc, nested, "b", "background:red", "y");
        let annotation = element(doc, outer, "rt", "display:ruby-text;font-size:5px");
        ids.1 = text_element(doc, annotation, "span", "background:red", "x");
    });
    let (mut doc, cascade, root) = (fixture.doc, fixture.cascade, fixture.root);
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    let pieces = doc.nodes[root].ifc_inline_boxes().expect("pieces");
    let span = pieces
        .iter()
        .find(|piece| piece.node == ids.1)
        .expect("annotation piece");
    assert_eq!(span.line, 0);
    assert!(span.border_box.width > 0.0, "{span:?}");
    let base = pieces
        .iter()
        .find(|piece| piece.node == ids.0)
        .expect("base piece");
    assert!(
        span.border_box.y + span.border_box.height <= base.border_box.y,
        "{pieces:?}"
    );
    assert_eq!(
        pieces
            .iter()
            .filter(|piece| doc.nodes[piece.node].tag_name() == Some("b"))
            .count(),
        1,
        "{pieces:?}"
    );
}

#[test]
fn content_without_a_ruby_mapping_falls_back() {
    let unplain: [fn(&mut Document, usize); 7] = [
        // An atomic inline.
        |doc, ruby| {
            element(doc, ruby, "span", "display:inline-block");
        },
        // An atomic inline in an annotation of a text container.
        |doc, ruby| {
            let rtc = element(doc, ruby, "rtc", "display:ruby-text-container");
            let annotation = element(doc, rtc, "rt", "display:ruby-text");
            element(doc, annotation, "span", "display:inline-block");
        },
        // An atomic inline in a base of a base container.
        |doc, ruby| {
            let rbc = element(doc, ruby, "rbc", "display:ruby-base-container");
            let base = element(doc, rbc, "rb", "display:ruby-base");
            element(doc, base, "span", "display:inline-block");
        },
        // A nested ruby that is not plain.
        |doc, ruby| {
            let inner = element(doc, ruby, "ruby", "display:ruby");
            element(doc, inner, "span", "display:inline-block");
        },
        // An annotation in a base container.
        |doc, ruby| {
            let rbc = element(doc, ruby, "rbc", "display:ruby-base-container");
            rt(doc, rbc, "x");
        },
        // A base in a text container.
        |doc, ruby| {
            let rtc = element(doc, ruby, "rtc", "display:ruby-text-container");
            rb(doc, rtc, "a");
        },
        // A block inside a base.
        |doc, ruby| {
            let base = rb(doc, ruby, "a");
            element(doc, base, "div", "display:block");
        },
    ];
    for build in unplain {
        let mut container = 0;
        let fixture = block_fixture("", |doc, root| {
            container = ruby(doc, root);
            build(doc, container);
        });
        assert!(plan(&fixture.doc, &fixture.cascade, container).is_none());
    }
}

#[test]
fn generated_content_falls_back() {
    for sheet in [
        "ruby::before { content: 'g' }",
        "rt::after { content: 'g' }",
    ] {
        let mut container = 0;
        let fixture = sheet_fixture(sheet, "", |doc, root| {
            container = ruby(doc, root);
            rb(doc, container, "a");
            rt(doc, container, "x");
        });
        assert!(plan(&fixture.doc, &fixture.cascade, container).is_none());
    }
}

#[test]
fn the_first_letter_continues_after_annotations_in_the_base_text() {
    let mut text = 0;
    let fixture = sheet_fixture("div::first-letter { font-size: 20px }", "", |doc, root| {
        let container = ruby(doc, root);
        rb(doc, container, "(");
        rt(doc, container, "x");
        text = doc.append_text(root, "AB");
    });
    let runs = runs(fixture);
    let xs: Vec<f32> = runs
        .iter()
        .filter(|run| run.0 == text)
        .map(|run| run.2.0)
        .collect();
    // `A` completes the first letter at 20px; `B` follows it at 10px.
    assert_eq!(xs.len(), 2, "{runs:?}");
    assert_eq!(xs[1] - xs[0], 20.0, "{runs:?}");
}

#[test]
fn a_forced_break_in_a_base_ends_the_first_letter() {
    let mut text = 0;
    let fixture = sheet_fixture("div::first-letter { font-size: 20px }", "", |doc, root| {
        let container = ruby(doc, root);
        let base = element(doc, container, "rb", "display:ruby-base");
        element(doc, base, "br", "");
        text = doc.append_text(base, "AB");
        rt(doc, container, "x");
    });
    let runs = runs(fixture);
    let xs: Vec<f32> = runs
        .iter()
        .filter(|run| run.0 == text)
        .map(|run| run.2.0)
        .collect();
    // `A` is on the second formatted line: no first letter splits `AB`.
    assert_eq!(xs, [0.0], "{runs:?}");
}

#[test]
fn annotation_text_owns_the_line_it_is_placed_over() {
    let mut text = 0;
    let fixture = block_fixture("line-height:20px", |doc, root| {
        let container = ruby(doc, root);
        rb(doc, container, "a");
        let annotation = rt(doc, container, "x");
        text = doc.nodes[annotation].children[0];
    });
    let (mut doc, cascade, root) = (fixture.doc, fixture.cascade, fixture.root);
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    let lines = doc.ifc_text_lines(text).expect("annotation lines");
    assert_eq!(lines.root, root);
    assert_eq!(
        lines.lines.iter().map(|line| line.line).collect::<Vec<_>>(),
        [0]
    );
    let by_node = doc.ifc_text_lines_by_node(root);
    assert_eq!(by_node.get(&text), Some(&lines));
}

#[test]
fn skipped_children_are_left_out() {
    let plan = plan_of(|doc, ruby| {
        rb(doc, ruby, "a");
        element(doc, ruby, "rb", "display:none");
        doc.append_comment(Some(ruby), "c");
        rt(doc, ruby, "x");
    });
    assert_eq!(plan, ["|rb(a)|[rt(x)]"]);
}

#[test]
fn annotation_levels_map_their_visibility_and_position() {
    assert_eq!(
        visibility(Visibility::Hidden),
        shodo::RubyVisibility::Hidden
    );
    assert_eq!(
        visibility(Visibility::Collapse),
        shodo::RubyVisibility::Collapse
    );
    assert_eq!(
        visibility(Visibility::Visible),
        shodo::RubyVisibility::Visible
    );
    assert_eq!(position(RubyPosition::Over), shodo::RubyPosition::Over);
    assert_eq!(position(RubyPosition::Under), shodo::RubyPosition::Under);
    assert_eq!(
        position(RubyPosition::InterCharacter),
        shodo::RubyPosition::InterCharacter
    );
}

/// Lay the paragraph out and return each run's text owner, whether it is an
/// annotation, and its first glyph's position.
fn runs(fixture: Fixture) -> Vec<(usize, bool, (f32, f32))> {
    let (mut doc, cascade, root) = (fixture.doc, fixture.cascade, fixture.root);
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    let lines = PositionedLines::new(&doc, &cascade, root, None).expect("lines");
    lines
        .lines()
        .flat_map(|line| {
            line.runs
                .iter()
                .filter_map(|run| {
                    let glyph = run.glyphs.first()?;
                    Some((run.owner, run.annotation.is_some(), (glyph.x, glyph.y)))
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn annotations_sit_over_their_bases() {
    let mut ids = (0, 0);
    let fixture = block_fixture("line-height:20px", |doc, root| {
        let container = ruby(doc, root);
        let base = rb(doc, container, "aa");
        let annotation = rt(doc, container, "xx");
        ids = (
            doc.nodes[base].children[0],
            doc.nodes[annotation].children[0],
        );
    });
    let runs = runs(fixture);
    let base = runs.iter().find(|run| run.0 == ids.0).expect("base run");
    let annotation = runs.iter().find(|run| run.0 == ids.1).expect("annotation");
    assert!(!base.1);
    assert!(annotation.1);
    assert!(annotation.2.1 < base.2.1, "{runs:?}");
}

#[test]
fn hidden_annotations_draw_only_their_visible_descendants() {
    let mut ids = (0, 0, 0);
    let fixture = block_fixture("line-height:20px", |doc, root| {
        let outer = ruby(doc, root);
        let inner = ruby(doc, outer);
        rb(doc, inner, "a");
        let nested = rt(doc, inner, "y");
        let hidden = text_element(doc, outer, "rt", "display:ruby-text;visibility:hidden", "x");
        let visible = text_element(doc, hidden, "span", "visibility:visible", "v");
        ids = (
            doc.nodes[nested].children[0],
            doc.nodes[hidden].children[0],
            doc.nodes[visible].children[0],
        );
    });
    let runs = runs(fixture);
    assert!(runs.iter().any(|run| run.0 == ids.0 && run.1), "{runs:?}");
    assert!(!runs.iter().any(|run| run.0 == ids.1), "{runs:?}");
    assert!(runs.iter().any(|run| run.0 == ids.2 && run.1), "{runs:?}");
}

#[test]
fn rubies_with_anonymous_parts_and_intra_level_spaces_lay_out() {
    let mut ids = Vec::new();
    let fixture = block_fixture("line-height:20px", |doc, root| {
        doc.append_text(root, "a");
        let container = ruby(doc, root);
        element(doc, container, "i", "float:left;width:5px;height:5px");
        doc.append_text(container, " b ");
        let span = element(doc, container, "span", "position:relative;left:1px");
        doc.append_text(span, "c");
        doc.append_text(container, " ");
        doc.append_element(
            Some(container),
            "br",
            Style::default(),
            Some("display:inline"),
        );
        let rtc = element(doc, container, "rtc", "display:ruby-text-container");
        rt(doc, rtc, "x");
        doc.append_text(rtc, " ");
        rt(doc, rtc, "y");
        doc.append_text(container, " ");
        let base = rb(doc, container, "d");
        ids.push(doc.nodes[base].children[0]);
    });
    let runs = runs(fixture);
    assert!(runs.iter().any(|run| run.0 == ids[0] && !run.1), "{runs:?}");
    assert!(runs.iter().filter(|run| run.1).count() >= 2, "{runs:?}");
}

#[test]
fn positioned_annotations_are_set_aside() {
    let mut ids = Vec::new();
    let fixture = block_fixture("", |doc, root| {
        let container = ruby(doc, root);
        rb(doc, container, "a");
        rt(doc, container, "x");
        ids.push(text_element(
            doc,
            container,
            "rt",
            "display:ruby-text;position:absolute",
            "y",
        ));
        let rtc = element(doc, container, "rtc", "display:ruby-text-container");
        ids.push(text_element(
            doc,
            rtc,
            "rt",
            "display:ruby-text;position:absolute",
            "z",
        ));
        let rbc = element(doc, container, "rbc", "display:ruby-base-container");
        ids.push(text_element(
            doc,
            rbc,
            "rb",
            "display:ruby-base;position:absolute",
            "b",
        ));
        rb(doc, rbc, "c");
        ids.insert(0, container);
    });
    let container = ids.remove(0);
    let plan = plan(&fixture.doc, &fixture.cascade, container).expect("plain ruby");
    assert_eq!(plan.out_of_flow, [(1, ids[0]), (1, ids[1]), (2, ids[2])]);
    assert_eq!(
        describe(&fixture.doc, &fixture.cascade, &plan),
        ["|rb(a)|[rt(x)]; rtc[]", "|rb(c)|"]
    );
}

#[test]
fn out_of_flow_boxes_are_anchored_between_segments_in_source_order() {
    let mut ids = Vec::new();
    let fixture = block_fixture("line-height:20px", |doc, root| {
        let container = ruby(doc, root);
        ids.push(container);
        ids.push(element(
            doc,
            container,
            "i",
            "float:left;width:5px;height:5px",
        ));
        doc.append_text(container, "a");
        rt(doc, container, "x");
        ids.push(element(doc, container, "i", "position:absolute"));
        doc.append_text(container, "b");
        let inner = ruby(doc, container);
        rb(doc, inner, "c");
        ids.push(element(doc, inner, "i", "position:absolute"));
        rt(doc, container, "y");
        ids.push(element(
            doc,
            container,
            "i",
            "float:right;width:5px;height:5px",
        ));
    });
    let container = ids.remove(0);
    let plan = plan(&fixture.doc, &fixture.cascade, container).expect("plain ruby");
    assert_eq!(
        plan.out_of_flow,
        [(0, ids[0]), (1, ids[1]), (2, ids[2]), (2, ids[3])]
    );
    let runs = runs(fixture);
    assert!(runs.iter().filter(|run| run.1).count() >= 2, "{runs:?}");
}

#[test]
fn deeply_nested_content_falls_back() {
    let mut container = 0;
    let fixture = block_fixture("", |doc, root| {
        container = ruby(doc, root);
        let mut parent = element(doc, container, "rb", "display:ruby-base");
        for _ in 0..=MAX_DEPTH {
            parent = element(doc, parent, "span", "display:inline");
        }
        doc.append_text(parent, "a");
    });
    assert!(plan(&fixture.doc, &fixture.cascade, container).is_none());
}

/// The number of lines of a paragraph of two ruby segments, `aa` over `x`
/// and `bb` over `y`, laid out with a continuation break at the text that
/// `at` picks from the gap text, the second base's text and its offset.
fn lines_with_break(at: impl FnOnce(usize, usize) -> (usize, u32)) -> usize {
    let mut texts = (0, 0);
    let fixture = block_fixture("line-height:20px", |doc, root| {
        let container = ruby(doc, root);
        rb(doc, container, "aa");
        rt(doc, container, "x");
        let gap = doc.append_text(container, "  ");
        let base = rb(doc, container, "bb");
        rt(doc, container, "y");
        texts = (gap, doc.nodes[base].children[0]);
    });
    let (mut doc, cascade, root) = (fixture.doc, fixture.cascade, fixture.root);
    doc.set_continuation_break(Some(at(texts.0, texts.1)));
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    PositionedLines::new(&doc, &cascade, root, None)
        .expect("lines")
        .lines()
        .count()
}

#[test]
fn continuation_breaks_start_lines_at_segments_and_in_the_line_text() {
    assert_eq!(lines_with_break(|_, base| (base, 0)), 2);
    assert_eq!(lines_with_break(|gap, _| (gap, 0)), 2);
    assert_eq!(lines_with_break(|gap, _| (gap, 1)), 2);
    // A base is not broken inside.
    assert_eq!(lines_with_break(|_, base| (base, 1)), 1);
}

#[test]
fn segments_without_text_are_left_out() {
    let mut ids = (0, 0);
    let fixture = block_fixture("line-height:20px", |doc, root| {
        let container = ruby(doc, root);
        element(doc, container, "rb", "display:ruby-base");
        element(doc, container, "rt", "display:ruby-text");
        doc.append_text(container, " ");
        let base = rb(doc, container, "a");
        let annotation = rt(doc, container, "x");
        ids = (
            doc.nodes[base].children[0],
            doc.nodes[annotation].children[0],
        );
    });
    let runs = runs(fixture);
    assert!(runs.iter().any(|run| run.0 == ids.0 && !run.1), "{runs:?}");
    assert!(runs.iter().any(|run| run.0 == ids.1 && run.1), "{runs:?}");
}

#[test]
fn containers_skip_what_renders_nothing_and_split_text_runs() {
    let plan = plan_of(|doc, ruby| {
        let rbc = element(doc, ruby, "rbc", "display:ruby-base-container");
        doc.append_comment(Some(rbc), "c");
        rb(doc, rbc, "a");
        let rtc = element(doc, ruby, "rtc", "display:ruby-text-container");
        doc.append_comment(Some(rtc), "c");
        rt(doc, rtc, "x");
        doc.append_text(rtc, "z");
        rt(doc, rtc, "y");
    });
    assert_eq!(plan, ["|rb(a)|rtc[rt(x) (z) rt(y)]"]);
}

#[test]
fn spaced_bases_text_containers_and_nested_annotations_lay_out() {
    let mut ids = Vec::new();
    let fixture = block_fixture("line-height:40px", |doc, root| {
        let container = ruby(doc, root);
        element(doc, container, "i", "position:absolute");
        let first = rb(doc, container, "a");
        doc.append_comment(Some(first), "c");
        doc.append_text(container, " ");
        rb(doc, container, "b");
        rt(doc, container, "x");
        rt(doc, container, "y");
        let rtc = element(doc, container, "rtc", "display:ruby-text-container");
        ids.push(doc.append_text(rtc, "w"));
        let annotation = element(doc, container, "rt", "display:ruby-text");
        let hidden = text_element(
            doc,
            annotation,
            "span",
            "display:inline;visibility:hidden",
            "h",
        );
        ids.push(doc.nodes[hidden].children[0]);
        let inner = ruby(doc, annotation);
        rb(doc, inner, "v");
        let nested = rt(doc, inner, "n");
        ids.push(doc.nodes[nested].children[0]);
    });
    let runs = runs(fixture);
    assert!(runs.iter().any(|run| run.0 == ids[0] && run.1), "{runs:?}");
    assert!(!runs.iter().any(|run| run.0 == ids[1]), "{runs:?}");
    assert!(runs.iter().any(|run| run.0 == ids[2] && run.1), "{runs:?}");
}

#[test]
fn rubies_that_lay_out_their_own_content_keep_their_annotations_over_the_bases() {
    for extra in ["display:flex", "position:relative", "float"] {
        let mut ids = (0, 0, 0);
        let root_style = if extra == "float" { "" } else { extra };
        let fixture = block_fixture(&format!("line-height:20px;{root_style}"), |doc, root| {
            let style = match extra {
                "display:flex" => "display:ruby;line-height:20px",
                "float" => "display:ruby;line-height:20px;float:left",
                _ => "display:ruby;line-height:20px;position:absolute",
            };
            let container = element(doc, root, "ruby", style);
            let base = rb(doc, container, "aa");
            let annotation = rt(doc, container, "xx");
            ids = (
                container,
                doc.nodes[base].children[0],
                doc.nodes[annotation].children[0],
            );
        });
        let (mut doc, cascade) = (fixture.doc, fixture.cascade);
        layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
        let lines = PositionedLines::new(&doc, &cascade, ids.0, None).expect("ruby root");
        let runs: Vec<_> = lines
            .lines()
            .flat_map(|line| {
                line.runs
                    .iter()
                    .map(|run| (run.owner, run.annotation.is_some(), run.glyphs[0].y))
                    .collect::<Vec<_>>()
            })
            .collect();
        let base = runs.iter().find(|run| run.0 == ids.1).expect("base");
        let annotation = runs.iter().find(|run| run.0 == ids.2).expect("annotation");
        assert!(!base.1 && annotation.1, "{extra}: {runs:?}");
        assert!(annotation.2 < base.2, "{extra}: {runs:?}");
    }
}
