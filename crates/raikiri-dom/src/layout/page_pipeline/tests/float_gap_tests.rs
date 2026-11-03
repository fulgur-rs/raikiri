use super::*;

/// `html > body > spacer, float, root > text`: a paragraph of 10px Ahem lines
/// in a box as wide as the left float before it, so the float puts its first
/// line below itself.
fn paragraph_after_a_float(
    text: &str,
    spacer_height: f32,
    float_height: f32,
    add_boxes: impl FnOnce(&mut Document, usize),
) -> (Document, CascadeResult, usize, usize, usize) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let spacer_css = format!("display:block;height:{spacer_height}px");
    doc.append_element(Some(body), "div", Some(&spacer_css));
    let float_css = format!("display:block;float:left;width:40px;height:{float_height}px");
    doc.append_element(Some(body), "div", Some(&float_css));
    let root = doc.append_element(
        Some(body),
        "div",
        Some("display:block;font-family:Ahem;font-size:10px;line-height:10px;width:40px"),
    );
    let text = doc.append_text(root, text);
    add_boxes(&mut doc, root);
    doc.mark_in_document_flags();
    let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    doc.set_font_collection_with_limits(
        crate::layout::test_support::ifc_ahem_fonts(),
        shodo::limits::Limits::default(),
    );
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
    (doc, cascade, body, root, text)
}

/// Keeps the lines of `text`, the first text of the paragraph `root`, together
/// at the page end 100px down. The floats before the paragraph end at 95, and
/// its first line is below them, from 95 to 105. Returns the state, the
/// position of the text and the document after the check, and whether the
/// check ran to the end: `control` can stop it at the page the lines move to.
fn keep_after_a_float(
    control: &PageLayoutControl<'_>,
    doc: Document,
    cascade: &CascadeResult,
    body: usize,
    root: usize,
    text: usize,
) -> (
    Result<(), pagination_state::Halt>,
    PaginationState,
    pagination_state::TextPosition,
    Document,
) {
    let mut doc = doc;
    let mut lines = TextLineIndex::default();
    lines.prepare_for(&doc, text, || Ok(())).unwrap();
    let pages = PageOrigins::new(&[], 100.0);
    let candidate = PageCandidate {
        node_id: text,
        raw_y: 95.0,
        height: 10.0,
        context: text_candidate_context(candidates::CandidateFlowContext::default()),
        page_name: None,
        named_flex_context: None,
        ifc_root: Some((root, 10.0)),
    };
    let entry = pagination_state::CandidateEntry {
        name_context_seen: false,
        comparison_page_name: None,
        keep_local_page_type: false,
        page_before_candidate: 0,
        ifc_root: None,
        participates_in_margin_adjacency: false,
        uses_positioned_coordinates: false,
    };
    let env = FlowEnv {
        cascade,
        body_id: body,
        control,
        page_origins: &pages,
        text_lines: &lines,
        roots: candidates::FlowRoots::new(&doc, body),
        root_flow_offset: 0.0,
        content_width: 800.0,
        avoided_breaks: &AvoidedBreaks::default(),
        insets: PageContentInsets::default(),
        page_box: page_box_800x600(),
    };
    let mut state = pagination_state_before_first_candidate(doc.nodes.len());
    let mut session = crate::layout::context::LayoutSession::resumed(&mut doc, cascade);
    let blocks = pagination_state::LineBlocks::find(
        &session,
        &env,
        &candidate,
        false,
        &state.checked_paragraphs,
    );
    let mut position = pagination_state::TextPosition {
        effective_y: 95.0,
        height: 10.0,
        first_line_shift: 0.0,
    };
    let kept = state.keep_lines_together(
        &mut session,
        &env,
        &candidate,
        &entry,
        &blocks,
        &mut position,
    );
    drop(session);
    (kept, state, position, doc)
}

#[test]
fn a_first_line_the_floats_put_down_moves_alone_to_the_next_page() {
    let (doc, cascade, body, root, text) = paragraph_after_a_float("aaaa", 85.0, 10.0, |_, _| {});
    assert_eq!(doc.nodes[root].layout.location.y, 85.0);
    assert_eq!(doc.nodes[root].layout.size.height, 20.0);
    let (kept, state, position, doc) = keep_after_a_float(
        &PageLayoutControl::new(None),
        doc,
        &cascade,
        body,
        root,
        text,
    );
    kept.unwrap();
    // The line moves down to the page start, 5px, and the text with it. The
    // block stays and grows by the move; the flow after it takes that growth
    // once it leaves the paragraph.
    assert_eq!(
        state.deferred_shifts.line_splits,
        [LineSplit {
            root,
            first_moved: 0,
            shift: 5.0,
            growth: 5.0,
            page: 1,
        }]
    );
    assert_eq!(
        (
            position.effective_y,
            position.first_line_shift,
            position.height
        ),
        (100.0, 5.0, 10.0)
    );
    assert_eq!(state.flow_shift, 0.0);
    assert_eq!(doc.nodes[root].layout.location.y, 85.0);
    assert_eq!(doc.nodes[root].layout.size.height, 25.0);
}

#[test]
fn a_first_line_that_moves_to_a_page_past_the_limit_halts_the_layout() {
    let (doc, cascade, body, root, text) = paragraph_after_a_float("aaaa", 85.0, 10.0, |_, _| {});
    // Geometry discovery stops at the first page, which the line moves to.
    let discovery = PageLayoutControl::for_geometry_discovery(Some(1));
    let (kept, state, ..) = keep_after_a_float(&discovery, doc, &cascade, body, root, text);
    assert!(matches!(kept, Err(pagination_state::Halt::PageLimit)));
    assert!(state.deferred_shifts.line_splits.is_empty());
}

#[test]
fn a_whole_paragraph_move_discards_the_inherited_float_gap() {
    // An inline box among the lines, or a float of the paragraph's own, keeps
    // the lines from moving down inside the box: it moves whole.
    for (name, css) in [
        ("inline box", "display:inline-block;width:10px;height:10px"),
        ("float", "display:block;float:left;width:10px;height:10px"),
    ] {
        let (doc, cascade, body, root, text) =
            paragraph_after_a_float("aaaa", 85.0, 10.0, |doc, root| {
                doc.append_element(Some(root), "span", Some(css));
            });
        let (kept, state, position, doc) = keep_after_a_float(
            &PageLayoutControl::new(None),
            doc,
            &cascade,
            body,
            root,
            text,
        );
        kept.unwrap();
        assert_eq!(
            state.deferred_shifts.line_splits,
            [LineSplit {
                root,
                first_moved: 0,
                shift: -10.0,
                growth: -10.0,
                page: 1,
            }],
            "{name}"
        );
        assert_eq!(
            (
                state.flow_shift,
                position.effective_y,
                position.first_line_shift
            ),
            (15.0, 100.0, -10.0),
            "{name}"
        );
        assert_eq!(doc.nodes[root].layout.location.y, 100.0, "{name}");
    }
}

#[test]
fn a_plain_block_gives_the_floats_before_it_the_first_line_its_text_holds() {
    // Whether the floats put the line of `text`, `offset` down in `root`.
    let put_down = |doc: &Document, root: usize, text: usize, offset: f32| {
        let mut lines = TextLineIndex::default();
        lines.prepare_for(doc, text, || Ok(())).unwrap();
        floats_put_first_line_down(doc, root, lines.get(text), offset)
    };
    // A block of text, some way down: the floats before it put the line there.
    let (doc, _, _, root, text) = unpaginated_paragraph_after_a_spacer("aaaa bbbb", "", 0.0);
    assert!(put_down(&doc, root, text, 20.0));
    // The line sits at the top of the content, or within a rounding error of it.
    assert!(!put_down(&doc, root, text, 0.0));
    assert!(!put_down(&doc, root, text, 0.5));
    // A text without lines.
    assert!(!floats_put_first_line_down(&doc, root, None, 20.0));
    // A box the floats do not reach into: it does not lay out in their flow.
    for css in ["display:inline-block", "writing-mode:vertical-rl"] {
        let (doc, _, _, root, text) = unpaginated_paragraph_after_a_spacer("aaaa bbbb", css, 0.0);
        assert!(!put_down(&doc, root, text, 20.0), "{css}");
    }
    // A text that starts on a later line does not hold the first one.
    let mut second = 0;
    let (doc, _, _, root, _) =
        unpaginated_paragraph_with_boxes_after_a_spacer("aaaa ", "", 0.0, |doc, root| {
            second = doc.append_text(root, "bbbb");
        });
    assert!(!put_down(&doc, root, second, 20.0));
}
