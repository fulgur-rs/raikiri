use super::*;
use crate::layout::ifc::test_support::{Fixture, ahem_fonts, sheet_fixture};
use shodo::limits::{LimitExceeded, LimitKind, Limits};
use shodo::{AtomicSizes, LayoutContext};

fn paragraph_builder(
    fixture: &Fixture,
    fonts: &FontCollection,
    limits: &Limits,
) -> ParagraphBuilder {
    let cv = &fixture.cascade.computed[fixture.root];
    let inline = styled(&fixture.doc, &fixture.cascade, cv, fixture.root, fonts).unwrap();
    let style = style::paragraph_style(cv, fixture.root, inline).unwrap();
    ParagraphBuilder::new(&style, limits)
}

fn first_text(fixture: &Fixture) -> usize {
    fixture.doc.get_node(fixture.root).unwrap().children[0]
}

fn glyph_runs(line: &shodo::Line) -> impl Iterator<Item = shodo::GlyphRunView<'_>> {
    line.fragments().filter_map(|fragment| match fragment {
        shodo::Fragment::GlyphRun(run) => Some(run),
        _ => None,
    })
}

#[test]
fn existing_builder_budget_error_is_preserved_before_first_letter_selection() {
    let fixture = sheet_fixture("div::first-letter{font-size:20px}", "", |doc, root| {
        doc.append_text(root, "A");
    });
    let fonts = ahem_fonts();
    let limits = Limits {
        max_text_bytes: Some(0),
        ..Limits::default()
    };
    let source = TextSource::Dom {
        node: NodeId(first_text(&fixture) as u64),
        offset: 0,
    };
    let mut builder = paragraph_builder(&fixture, &fonts, &limits);
    builder.push_text(source, "A");
    let expected = LimitExceeded {
        kind: LimitKind::TextBytes,
        limit: 0,
        actual: 1,
    };
    assert_eq!(builder.error(), Some(expected));
    let mut letter = FirstLetter::new(&fixture.doc, &fixture.cascade, fixture.root, &limits);
    let error = letter
        .push(
            &mut builder,
            &fixture.doc,
            &fixture.cascade,
            source,
            &fixture.cascade.computed[fixture.root],
            "A",
            &fonts,
        )
        .unwrap_err();
    assert!(matches!(error, IfcError::Limit(actual) if actual == expected));
    assert_eq!(builder.error(), Some(expected));
    assert!(letter.styles.is_empty());
}

#[test]
fn first_letter_source_offsets_fail_explicitly_instead_of_wrapping() {
    for text in ["A", " A"] {
        let fixture = sheet_fixture("div::first-letter{font-size:20px}", "", |doc, root| {
            doc.append_text(root, text);
        });
        let fonts = ahem_fonts();
        let limits = Limits::default();
        let mut builder = paragraph_builder(&fixture, &fonts, &limits);
        let mut letter = FirstLetter::new(&fixture.doc, &fixture.cascade, fixture.root, &limits);
        let error = letter
            .push(
                &mut builder,
                &fixture.doc,
                &fixture.cascade,
                TextSource::Dom {
                    node: NodeId(first_text(&fixture) as u64),
                    offset: u32::MAX,
                },
                &fixture.cascade.computed[fixture.root],
                text,
                &fonts,
            )
            .unwrap_err();
        assert!(matches!(error, IfcError::Unsupported {
            node,
            reason: "first-letter source offset exceeds shodo's address range",
        } if node == fixture.root));
        assert!(letter.styles.is_empty());
        assert!(builder.error().is_none());
        assert_eq!(
            builder
                .build(&mut LayoutContext::new(), &fonts)
                .unwrap()
                .text(),
            ""
        );
    }
}

#[test]
fn opening_the_letter_inline_box_reports_the_item_budget_boundary() {
    let fixture = sheet_fixture("div::first-letter{font-size:20px}", "", |doc, root| {
        let span = doc.append_element(
            Some(root),
            "span",
            taffy::Style::default(),
            Some("display:inline"),
        );
        doc.append_text(span, "A");
    });
    let fonts = ahem_fonts();
    let limits = Limits {
        max_items: Some(1),
        ..Limits::default()
    };
    let span = first_text(&fixture);
    let text = fixture.doc.get_node(span).unwrap().children[0];
    let source = TextSource::Dom {
        node: NodeId(text as u64),
        offset: 0,
    };
    let mut builder = paragraph_builder(&fixture, &fonts, &limits);
    let cv = &fixture.cascade.computed[span];
    let inline = styled(&fixture.doc, &fixture.cascade, cv, span, &fonts).unwrap();
    let edges = style::inline_edges(cv, span, &fonts).unwrap();
    builder.open_inline(NodeId(span as u64), &inline, edges);
    assert!(builder.error().is_none());
    let mut letter = FirstLetter::new(&fixture.doc, &fixture.cascade, fixture.root, &limits);
    let error = letter
        .push(
            &mut builder,
            &fixture.doc,
            &fixture.cascade,
            source,
            cv,
            "A",
            &fonts,
        )
        .unwrap_err();
    let expected = LimitExceeded {
        kind: LimitKind::Items,
        limit: 1,
        actual: 2,
    };
    assert!(matches!(error, IfcError::Limit(actual) if actual == expected));
    assert_eq!(builder.error(), Some(expected));
    assert!(letter.styles.is_empty());
}

#[test]
fn one_generated_before_letter_uses_its_generated_source_without_dom_lookahead() {
    let fixture = sheet_fixture(
        "div::before{content:'A'} div::first-letter{font-size:20px}",
        "",
        |_, _| {},
    );
    let fonts = ahem_fonts();
    let limits = Limits::default();
    let before = generated_node_id(fixture.root, PseudoElem::Before);
    let mut builder = paragraph_builder(&fixture, &fonts, &limits);
    let mut letter = FirstLetter::new(&fixture.doc, &fixture.cascade, fixture.root, &limits);
    letter
        .push(
            &mut builder,
            &fixture.doc,
            &fixture.cascade,
            TextSource::Generated {
                node: NodeId(before as u64),
            },
            &fixture.cascade.pseudo[&(StyleNodeId::new(fixture.root as u64), PseudoElem::Before)],
            "A",
            &fonts,
        )
        .unwrap();
    assert_eq!(letter.styles.len(), 1);
    assert_eq!(letter.styles[0].source_owner, before);
    assert_eq!(letter.styles[0].source_range, Some(0..1));
    let mut cx = LayoutContext::new();
    let paragraph = builder.build(&mut cx, &fonts).unwrap();
    assert_eq!(paragraph.text(), "A");
    let (options, _) = style::line_options(
        &fixture.cascade.computed[fixture.root],
        fixture.root,
        &fonts,
    )
    .unwrap();
    let lines = paragraph.break_all(&mut cx, &options, 100.0, &AtomicSizes::EMPTY);
    let run = glyph_runs(&lines[0]).next().unwrap();
    assert_eq!(run.font_size(), 20.0);
    assert_eq!(
        run.source(),
        Some(TextSource::Dom {
            node: NodeId(before as u64),
            offset: 0,
        })
    );
}

#[test]
fn empty_text_before_a_comment_split_grapheme_keeps_original_sources() {
    let fixture = sheet_fixture("div::first-letter{font-size:20px}", "", |doc, root| {
        doc.append_text(root, "");
        doc.append_text(root, "A");
        doc.append_comment(Some(root), "a grapheme may span original text nodes");
        doc.append_text(root, "\u{301}Y");
    });
    let children = &fixture.doc.get_node(fixture.root).unwrap().children;
    let fonts = ahem_fonts();
    let limits = Limits::default();
    let mut builder = paragraph_builder(&fixture, &fonts, &limits);
    let mut letter = FirstLetter::new(&fixture.doc, &fixture.cascade, fixture.root, &limits);
    for (&node, text) in [
        (&children[0], ""),
        (&children[1], "A"),
        (&children[3], "\u{301}Y"),
    ] {
        letter
            .push(
                &mut builder,
                &fixture.doc,
                &fixture.cascade,
                TextSource::Dom {
                    node: NodeId(node as u64),
                    offset: 0,
                },
                &fixture.cascade.computed[fixture.root],
                text,
                &fonts,
            )
            .unwrap();
    }
    assert_eq!(letter.styles.len(), 2);
    assert_eq!(letter.styles[0].source_owner, children[1]);
    assert_eq!(letter.styles[0].source_range, Some(0..1));
    assert_eq!(letter.styles[1].source_owner, children[3]);
    assert_eq!(letter.styles[1].source_range, Some(0..2));
    let mut cx = LayoutContext::new();
    let paragraph = builder.build(&mut cx, &fonts).unwrap();
    assert_eq!(paragraph.text(), "A\u{301}Y");
    let (options, _) = style::line_options(
        &fixture.cascade.computed[fixture.root],
        fixture.root,
        &fonts,
    )
    .unwrap();
    let lines = paragraph.break_all(&mut cx, &options, 100.0, &AtomicSizes::EMPTY);
    assert!(lines.iter().flat_map(glyph_runs).any(|run| run.source()
        == Some(TextSource::Dom {
            node: NodeId(children[3] as u64),
            offset: 2
        })
        && run.font_size() == 10.0));
}

#[test]
fn preserved_whitespace_before_an_inner_block_stops_ancestor_letter_selection() {
    for (extra, expected_size) in [("", 20.0), ("white-space:pre", 10.0)] {
        let fixture = sheet_fixture("div::first-letter{font-size:20px}", extra, |doc, root| {
            doc.append_text(root, " ");
            let block = doc.append_element(
                Some(root),
                "p",
                taffy::Style::default(),
                Some("display:block"),
            );
            doc.append_text(block, "A");
        });
        let block = fixture.doc.get_node(fixture.root).unwrap().children[1];
        let text = fixture.doc.get_node(block).unwrap().children[0];
        let fonts = ahem_fonts();
        let limits = Limits::default();
        let mut builder = paragraph_builder(&fixture, &fonts, &limits);
        let mut letter = FirstLetter::new(&fixture.doc, &fixture.cascade, block, &limits);
        letter
            .push(
                &mut builder,
                &fixture.doc,
                &fixture.cascade,
                TextSource::Dom {
                    node: NodeId(text as u64),
                    offset: 0,
                },
                &fixture.cascade.computed[block],
                "A",
                &fonts,
            )
            .unwrap();
        assert_eq!(letter.styles.len(), usize::from(extra.is_empty()));
        let mut cx = LayoutContext::new();
        let paragraph = builder.build(&mut cx, &fonts).unwrap();
        let (options, _) =
            style::line_options(&fixture.cascade.computed[block], block, &fonts).unwrap();
        let lines = paragraph.break_all(&mut cx, &options, 100.0, &AtomicSizes::EMPTY);
        let run = lines.iter().flat_map(glyph_runs).next().unwrap();
        assert_eq!(run.font_size(), expected_size);
        assert_eq!(
            run.source(),
            Some(TextSource::Dom {
                node: NodeId(text as u64),
                offset: 0,
            })
        );
    }
}

#[test]
fn whitespace_only_prefix_keeps_normal_source_until_the_adjacent_letter() {
    let fixture = sheet_fixture(
        "div::first-letter{font-size:20px}",
        "white-space:pre",
        |doc, root| {
            doc.append_text(root, " ");
            doc.append_comment(Some(root), "whitespace and letter keep their own sources");
            doc.append_text(root, "AB");
        },
    );
    let children = &fixture.doc.get_node(fixture.root).unwrap().children;
    let prefix = children[0];
    let letters = children[2];
    let fonts = ahem_fonts();
    let limits = Limits::default();
    let mut builder = paragraph_builder(&fixture, &fonts, &limits);
    let mut letter = FirstLetter::new(&fixture.doc, &fixture.cascade, fixture.root, &limits);
    let cv = &fixture.cascade.computed[fixture.root];
    letter
        .push(
            &mut builder,
            &fixture.doc,
            &fixture.cascade,
            TextSource::Dom {
                node: NodeId(prefix as u64),
                offset: 0,
            },
            cv,
            " ",
            &fonts,
        )
        .unwrap();
    assert!(letter.styles.is_empty());
    letter
        .push(
            &mut builder,
            &fixture.doc,
            &fixture.cascade,
            TextSource::Dom {
                node: NodeId(letters as u64),
                offset: 0,
            },
            cv,
            "AB",
            &fonts,
        )
        .unwrap();
    assert_eq!(letter.styles.len(), 1);
    assert_eq!(letter.styles[0].source_owner, letters);
    assert_eq!(letter.styles[0].source_range, Some(0..1));
    let mut cx = LayoutContext::new();
    let paragraph = builder.build(&mut cx, &fonts).unwrap();
    assert_eq!(paragraph.text(), " AB");
    let (options, _) = style::line_options(cv, fixture.root, &fonts).unwrap();
    let lines = paragraph.break_all(&mut cx, &options, 100.0, &AtomicSizes::EMPTY);
    for (node, offset, size) in [(prefix, 0, 10.0), (letters, 0, 20.0), (letters, 1, 10.0)] {
        assert!(lines.iter().flat_map(glyph_runs).any(|run| run.source()
            == Some(TextSource::Dom {
                node: NodeId(node as u64),
                offset,
            })
            && run.font_size() == size));
    }
}

#[test]
fn generated_before_propagates_the_floating_letter_error_through_projection() {
    let fixture = sheet_fixture(
        "div::before{content:'A'} div::first-letter{float:left;font-size:40px}",
        "",
        |doc, root| {
            doc.append_text(root, "B");
        },
    );
    let result = super::super::projection::project_ifc(
        &fixture.doc,
        &fixture.cascade,
        fixture.root,
        &mut LayoutContext::new(),
        &ahem_fonts(),
        &Limits::default(),
    );
    assert!(matches!(
        result,
        Err(IfcError::Unsupported {
            node,
            reason: "floating ::first-letter requires drop-cap box layout",
        }) if node == fixture.root
    ));
}

#[test]
fn adjacent_letter_selection_skips_hidden_and_out_of_flow_inline_subtrees() {
    for (tag, css) in [
        ("span", "display:none"),
        ("script", "display:inline"),
        ("span", "display:inline;position:absolute"),
        ("span", "display:inline;float:left"),
    ] {
        let mut owner = 0;
        let mut next = 0;
        let fixture = sheet_fixture("div::first-letter{color:red}", "", |doc, root| {
            owner = doc.append_text(root, "(");
            let skipped = doc.append_element(Some(root), tag, taffy::Style::default(), Some(css));
            doc.append_text(skipped, "Z");
            let span = doc.append_element(
                Some(root),
                "span",
                taffy::Style::default(),
                Some("display:inline"),
            );
            next = doc.append_text(span, "Ab");
        });
        let mut letter = FirstLetter::new(
            &fixture.doc,
            &fixture.cascade,
            fixture.root,
            &Limits::default(),
        );
        let range = letter
            .adjacent_range(
                &fixture.doc,
                &fixture.cascade,
                TextSource::Dom {
                    node: NodeId(owner as u64),
                    offset: 0,
                },
                "(",
                false,
                &super::super::projection::GeneratedCounters::default(),
            )
            .unwrap();
        assert_eq!(range, Some(0..1));
        assert_eq!(letter.continuation, VecDeque::from([(next, 0..1)]));
    }
}

#[test]
fn adjacent_letter_selection_stops_at_a_line_break() {
    for css in ["display:inline", "display:inline;visibility:hidden"] {
        let mut owner = 0;
        let fixture = sheet_fixture("div::first-letter{color:red}", "", |doc, root| {
            owner = doc.append_text(root, " ");
            doc.append_element(Some(root), "br", taffy::Style::default(), Some(css));
            doc.append_text(root, "Ab");
        });
        let mut letter = FirstLetter::new(
            &fixture.doc,
            &fixture.cascade,
            fixture.root,
            &Limits::default(),
        );
        let range = letter
            .adjacent_range(
                &fixture.doc,
                &fixture.cascade,
                TextSource::Dom {
                    node: NodeId(owner as u64),
                    offset: 0,
                },
                " ",
                false,
                &super::super::projection::GeneratedCounters::default(),
            )
            .unwrap();
        assert_eq!(range, None);
        assert!(letter.continuation.is_empty());
    }
}

#[test]
fn an_earlier_block_generated_box_blocks_a_later_owner_lookahead() {
    let mut owner = 0;
    let fixture = sheet_fixture(
        "div::first-letter{color:red} div::before{display:block;content:'X'}",
        "",
        |doc, root| {
            owner = doc.append_text(root, "Ab");
        },
    );
    let mut letter = FirstLetter::new(
        &fixture.doc,
        &fixture.cascade,
        fixture.root,
        &Limits::default(),
    );
    assert_eq!(
        letter
            .adjacent_range(
                &fixture.doc,
                &fixture.cascade,
                TextSource::Dom {
                    node: NodeId(owner as u64),
                    offset: 0
                },
                "Ab",
                false,
                &super::super::projection::GeneratedCounters::default(),
            )
            .unwrap(),
        None
    );
    assert!(letter.continuation.is_empty());
}

#[test]
fn generated_text_in_a_preceding_contents_sibling_blocks_ancestor_letter_styling() {
    let mut child = 0;
    let fixture = sheet_fixture(
        "div::first-letter{color:red} span::before{content:'X'}",
        "",
        |doc, root| {
            doc.append_element(
                Some(root),
                "span",
                taffy::Style::default(),
                Some("display:contents"),
            );
            child = doc.append_element(
                Some(root),
                "section",
                taffy::Style::default(),
                Some("display:block"),
            );
            doc.append_text(child, "Ab");
        },
    );
    let letter = FirstLetter::new(&fixture.doc, &fixture.cascade, child, &Limits::default());
    assert!(letter.origins.is_empty());
    assert!(!letter.pending);
}

#[test]
fn first_letter_predecessor_work_is_bounded_across_many_ifc_roots() {
    let mut roots = Vec::new();
    let fixture = sheet_fixture("div::first-letter{color:red}", "", |doc, parent| {
        for _ in 0..200 {
            doc.append_comment(Some(parent), "ignored");
        }
        for _ in 0..200 {
            let root = doc.append_element(
                Some(parent),
                "section",
                taffy::Style::default(),
                Some("display:block"),
            );
            doc.append_text(root, "A");
            roots.push(root);
        }
    });
    PREDECESSOR_VISITS.with(|visits| visits.set(0));
    let predecessors = PredecessorCache::default();
    for (index, root) in roots.into_iter().enumerate() {
        let letter = FirstLetter::new_with_predecessors(
            &fixture.doc,
            &fixture.cascade,
            root,
            &Limits::default(),
            &predecessors,
        );
        assert_eq!(
            letter.origins,
            if index == 0 {
                vec![fixture.root]
            } else {
                vec![]
            }
        );
    }
    let visited = PREDECESSOR_VISITS.with(|visits| visits.get());
    assert!(
        visited <= fixture.doc.node_count() * 3,
        "predecessor work {visited} exceeds the document-wide bound"
    );
}

#[test]
fn review_nested_first_lines_supply_the_full_letter_parent() {
    let fixture = sheet_fixture(
        "body::first-line{color:red} div::first-line{font-size:20px} div::first-letter{border:1px solid currentColor;font-size:2em}",
        "",
        |doc, root| {
            doc.append_text(root, "AB");
        },
    );
    let projected = super::super::projection::project_ifc(
        &fixture.doc,
        &fixture.cascade,
        fixture.root,
        &mut LayoutContext::new(),
        &ahem_fonts(),
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(projected.letter_styles.len(), 1);
    let cv = &projected.letter_styles[0].computed;
    assert_eq!((cv.color.r, cv.color.g, cv.color.b), (255, 0, 0));
    assert_eq!(
        cv.border.top.color,
        raikiri_style::property::BorderColor::CurrentColor
    );
    assert_eq!(cv.font_size.px(), 40.0);
}

#[test]
fn review_block_after_does_not_join_punctuation_on_the_previous_line() {
    for display in ["block", "inline"] {
        let fixture = sheet_fixture(
            &format!(
                "div::after{{display:{display};content:'A'}} div::first-letter{{font-size:20px}}"
            ),
            "",
            |doc, root| {
                doc.append_text(root, "(");
            },
        );
        let projected = super::super::projection::project_ifc(
            &fixture.doc,
            &fixture.cascade,
            fixture.root,
            &mut LayoutContext::new(),
            &ahem_fonts(),
            &Limits::default(),
        )
        .unwrap();
        assert_eq!(projected.letter_styles.is_empty(), display == "block");
    }
    let fixture = sheet_fixture(
        "div::before{display:block;content:'A'} div::first-letter{font-size:20px}",
        "",
        |doc, root| {
            doc.append_text(root, "B");
        },
    );
    let projected = super::super::projection::project_ifc(
        &fixture.doc,
        &fixture.cascade,
        fixture.root,
        &mut LayoutContext::new(),
        &ahem_fonts(),
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(projected.letter_styles.len(), 1);
    assert_eq!(
        projected.letter_styles[0].source_owner,
        generated_node_id(fixture.root, PseudoElem::Before)
    );
}

#[test]
fn review_atomic_generated_text_is_outside_the_parent_first_letter() {
    for display in [
        "inline-block",
        "inline-flex",
        "inline-grid",
        "inline-table",
        "inline",
        "contents",
    ] {
        for pseudo in ["before", "after"] {
            let sheet = format!(
                "div::{pseudo}{{display:{display};content:'A'}} div::first-letter{{font-size:20px}}"
            );
            let fixture = sheet_fixture(&sheet, "", |doc, root| {
                doc.append_text(root, if pseudo == "before" { "B" } else { "(" });
            });
            let projected = super::super::projection::project_ifc(
                &fixture.doc,
                &fixture.cascade,
                fixture.root,
                &mut LayoutContext::new(),
                &ahem_fonts(),
                &Limits::default(),
            )
            .unwrap();
            assert_eq!(
                projected.letter_styles.is_empty(),
                !matches!(display, "inline" | "contents"),
                "{pseudo} {display}"
            );
        }
    }
}

fn ancestor_letter_projection(fixture: &Fixture, root: usize) -> Vec<LetterStyle> {
    super::super::projection::project_ifc(
        &fixture.doc,
        &fixture.cascade,
        root,
        &mut LayoutContext::new(),
        &ahem_fonts(),
        &Limits::default(),
    )
    .unwrap()
    .letter_styles
}

#[test]
fn review_logical_float_predecessors_follow_the_projected_in_flow_blocks() {
    for (float, in_flow) in [
        ("inline-start", true),
        ("inline-end", true),
        ("none", true),
        ("left", false),
        ("right", false),
        ("footnote", false),
    ] {
        let mut roots = Vec::new();
        let mut texts = Vec::new();
        let fixture = sheet_fixture("div::first-letter{font-size:20px}", "", |doc, root| {
            for (text, css) in [
                ("A", format!("display:block;float:{float}")),
                ("B", "display:block".into()),
            ] {
                let child = doc.append_element(
                    Some(root),
                    "p",
                    taffy::Style::default(),
                    Some(css.as_str()),
                );
                roots.push(child);
                texts.push(doc.append_text(child, text));
            }
        });
        let first = ancestor_letter_projection(&fixture, roots[0]);
        let second = ancestor_letter_projection(&fixture, roots[1]);
        assert_eq!(first.len(), usize::from(in_flow), "{float} first block");
        assert_eq!(second.len(), usize::from(!in_flow), "{float} later block");
        let selected = if in_flow { &first } else { &second };
        assert_eq!(selected[0].source_owner, texts[usize::from(!in_flow)]);
        assert_eq!(selected[0].source_range, Some(0..1));
        assert_eq!(selected[0].computed.font_size.px(), 20.0);
    }
}

#[test]
fn review_pre_line_predecessor_spaces_match_anonymous_line_eligibility() {
    for (text, blocks) in [
        (" ", false),
        ("\t\u{000c}", false),
        ("", false),
        ("\n", true),
        ("\r\n", true),
        ("A", true),
        ("\u{00a0}", true),
    ] {
        let mut child = 0;
        let fixture = sheet_fixture(
            "div::first-letter{font-size:20px}",
            "white-space:pre-line",
            |doc, root| {
                doc.append_text(root, text);
                child = doc.append_element(
                    Some(root),
                    "p",
                    taffy::Style::default(),
                    Some("display:block"),
                );
                doc.append_text(child, "B");
            },
        );
        let selected = ancestor_letter_projection(&fixture, child);
        assert_eq!(selected.is_empty(), blocks, "{text:?}");
        if !blocks {
            assert_eq!(selected[0].source_range, Some(0..1));
            assert_eq!(selected[0].computed.font_size.px(), 20.0);
        }
    }
}

#[test]
fn review_empty_inline_predecessors_do_not_form_a_first_letter_barrier() {
    for (display, nested, contributes) in [
        ("inline", false, false),
        ("inline", true, false),
        ("contents", false, false),
        ("contents", true, false),
        ("block", false, true),
        ("inline-block", false, true),
        ("inline-flex", false, true),
        ("inline-grid", false, true),
        ("inline-table", false, true),
    ] {
        let mut child = 0;
        let fixture = sheet_fixture("div::first-letter{font-size:20px}", "", |doc, root| {
            let span = doc.append_element(
                Some(root),
                "span",
                taffy::Style::default(),
                Some(format!("display:{display}").as_str()),
            );
            if nested {
                doc.append_element(
                    Some(span),
                    "span",
                    taffy::Style::default(),
                    Some("display:inline"),
                );
            }
            child = doc.append_element(
                Some(root),
                "p",
                taffy::Style::default(),
                Some("display:block"),
            );
            doc.append_text(child, "A");
        });
        let selected = ancestor_letter_projection(&fixture, child);
        assert_eq!(
            selected.is_empty(),
            contributes,
            "{display} nested={nested}"
        );
        if !contributes {
            assert_eq!(selected[0].source_range, Some(0..1));
            assert_eq!(selected[0].computed.font_size.px(), 20.0);
        }
    }
}

#[test]
fn review_inline_predecessors_keep_text_generated_and_forced_line_barriers() {
    for (tag, content, generated) in [
        ("span", "A", false),
        ("span", "\u{00a0}", false),
        ("span", "", true),
        ("br", "", false),
        ("wbr", "", false),
        ("img", "", false),
    ] {
        let mut child = 0;
        let sheet = if generated {
            "div::first-letter{font-size:20px} span::before{content:'A'}"
        } else {
            "div::first-letter{font-size:20px}"
        };
        let fixture = sheet_fixture(sheet, "", |doc, root| {
            let inline = doc.append_element(
                Some(root),
                tag,
                taffy::Style::default(),
                Some("display:inline"),
            );
            doc.append_text(inline, content);
            child = doc.append_element(
                Some(root),
                "p",
                taffy::Style::default(),
                Some("display:block"),
            );
            doc.append_text(child, "B");
        });
        assert!(
            ancestor_letter_projection(&fixture, child).is_empty(),
            "{tag} {content:?} generated={generated}"
        );
    }
}

#[test]
fn review_empty_inline_subtree_work_stays_document_bounded() {
    let mut roots = Vec::new();
    let fixture = sheet_fixture("div::first-letter{color:red}", "", |doc, root| {
        let mut nested = root;
        for _ in 0..64 {
            nested = doc.append_element(
                Some(nested),
                "span",
                taffy::Style::default(),
                Some("display:inline"),
            );
            for _ in 0..8 {
                doc.append_element(
                    Some(nested),
                    "span",
                    taffy::Style::default(),
                    Some("display:inline"),
                );
            }
        }
        for _ in 0..200 {
            let block = doc.append_element(
                Some(root),
                "p",
                taffy::Style::default(),
                Some("display:block"),
            );
            doc.append_text(block, "A");
            roots.push(block);
        }
    });
    PREDECESSOR_VISITS.with(|visits| visits.set(0));
    let predecessors = PredecessorCache::default();
    for (index, root) in roots.into_iter().enumerate() {
        let letter = FirstLetter::new_with_predecessors(
            &fixture.doc,
            &fixture.cascade,
            root,
            &Limits::default(),
            &predecessors,
        );
        assert_eq!(
            letter.origins,
            if index == 0 {
                vec![fixture.root]
            } else {
                vec![]
            }
        );
    }
    let visits = PREDECESSOR_VISITS.with(|visits| visits.get());
    assert!(visits <= fixture.doc.node_count() * 3);
}

#[test]
fn review_generated_inline_predecessors_keep_before_after_atomic_barriers() {
    for pseudo in ["before", "after"] {
        for display in [
            "inline",
            "contents",
            "inline-block",
            "inline-flex",
            "inline-grid",
            "inline-table",
        ] {
            let mut child = 0;
            let fixture = sheet_fixture(
                &format!(
                    "div::first-letter{{font-size:20px}} span::{pseudo}{{content:'X';display:{display}}}"
                ),
                "",
                |doc, root| {
                    doc.append_element(
                        Some(root),
                        "span",
                        taffy::Style::default(),
                        Some("display:inline"),
                    );
                    child = doc.append_element(
                        Some(root),
                        "p",
                        taffy::Style::default(),
                        Some("display:block"),
                    );
                    doc.append_text(child, "A");
                },
            );
            assert!(
                ancestor_letter_projection(&fixture, child).is_empty(),
                "{pseudo} {display}"
            );
        }
    }
}

#[test]
fn review_empty_generated_ancestors_and_predecessors_keep_the_block_child_boundary() {
    for content in ["''", "attr(missing)", "no-open-quote", "no-close-quote"] {
        for ancestor in [true, false] {
            let target = if ancestor { "div" } else { "span" };
            let sheet = format!(
                "div::first-letter{{font-size:20px}} {target}::before{{content:{content}}}"
            );
            let mut child = 0;
            let fixture = sheet_fixture(&sheet, "", |doc, root| {
                if !ancestor {
                    doc.append_element(
                        Some(root),
                        "span",
                        taffy::Style::default(),
                        Some("display:contents"),
                    );
                }
                child = doc.append_element(
                    Some(root),
                    "p",
                    taffy::Style::default(),
                    Some("display:block"),
                );
                doc.append_text(child, "A");
            });
            let selected = ancestor_letter_projection(&fixture, child);
            assert_eq!(selected.len(), 0, "{content}: ancestor={ancestor}");
        }
    }
}

#[test]
fn review_preserve_spaces_converts_initial_segment_breaks_before_letter_selection() {
    for text in ["\nA", "\tA", "\r\nA"] {
        let fixture = sheet_fixture(
            "div::first-letter{font-size:20px}",
            "white-space-collapse:preserve-spaces",
            |doc, root| {
                doc.append_text(root, text);
            },
        );
        let selected = ancestor_letter_projection(&fixture, fixture.root);
        assert_eq!(selected.len(), 1, "{text:?}");
        assert_eq!(selected[0].source_owner, first_text(&fixture));
        assert_eq!(
            selected[0].source_range,
            Some((text.len() as u32 - 1)..text.len() as u32)
        );
    }
}

#[test]
fn review_resolved_nonempty_generated_predecessors_still_block_ancestor_letters() {
    for content in ["'B'", "attr(title)", "counter(section)", "open-quote"] {
        for ancestor in [true, false] {
            let target = if ancestor { "div" } else { "span" };
            let sheet = format!(
                "div::first-letter{{font-size:20px}} {target}::before{{content:{content}}}"
            );
            let mut child = 0;
            let fixture = sheet_fixture(&sheet, "", |doc, root| {
                let owner = if ancestor {
                    root
                } else {
                    doc.append_element(
                        Some(root),
                        "span",
                        taffy::Style::default(),
                        Some("display:contents"),
                    )
                };
                doc.set_element_attributes(owner, vec![("title".into(), "B".into())]);
                child = doc.append_element(
                    Some(root),
                    "p",
                    taffy::Style::default(),
                    Some("display:block"),
                );
                doc.append_text(child, "A");
            });
            assert!(ancestor_letter_projection(&fixture, child).is_empty());
        }
    }
}

#[test]
fn review_empty_atomic_generated_boxes_remain_first_letter_barriers() {
    for pseudo in ["before", "after"] {
        let fixture = sheet_fixture(
            &format!(
                "div::first-letter{{font-size:20px}} span::{pseudo}{{display:inline-block;content:attr(missing)}}"
            ),
            "",
            |doc, root| {
                doc.append_text(root, "(");
                doc.append_element(Some(root), "span", taffy::Style::default(), None::<&str>);
                doc.append_text(root, "A");
            },
        );
        assert!(ancestor_letter_projection(&fixture, fixture.root).is_empty());
    }
}

#[test]
fn review_empty_atomic_before_keeps_the_origins_letter_unselected() {
    for display in ["inline-block", "inline-flex", "inline-grid", "inline-table"] {
        let fixture = sheet_fixture(
            &format!(
                "div::first-letter{{font-size:20px}} div::before{{display:{display};content:attr(missing)}}"
            ),
            "",
            |doc, root| {
                doc.append_text(root, "A");
            },
        );
        assert!(
            ancestor_letter_projection(&fixture, fixture.root).is_empty(),
            "{display}"
        );
    }
}

#[test]
fn review_empty_ordinary_generated_text_keeps_adjacent_source_continuations() {
    for display in ["inline", "contents"] {
        for pseudo in ["before", "after"] {
            let mut owners = Vec::new();
            let fixture = sheet_fixture(
                &format!(
                    "div::first-letter{{font-size:20px}} span::{pseudo}{{display:{display};content:attr(missing)}}"
                ),
                "",
                |doc, root| {
                    owners.push(doc.append_text(root, "("));
                    doc.append_element(Some(root), "span", taffy::Style::default(), None::<&str>);
                    owners.push(doc.append_text(root, "A"));
                },
            );
            let selected = ancestor_letter_projection(&fixture, fixture.root);
            assert_eq!(selected.len(), 2, "{display}::{pseudo}");
            assert_eq!(selected[0].source_owner, owners[0]);
            assert_eq!(selected[1].source_owner, owners[1]);
            assert_eq!(selected[0].source_range, Some(0..1));
            assert_eq!(selected[1].source_range, Some(0..1));
        }
    }
}
