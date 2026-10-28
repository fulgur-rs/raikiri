use super::*;
use crate::fragment::{FragmentRect, LayoutFragment};
use crate::layout::test_support::{
    absolute_rect, ahem_paragraph_with, ahem_table_with_only_text, with_ahem,
};
use crate::{IfcTextLine, IfcTextLines};
use source_walk::{
    FragmentIndex, Paragraphs, WalkedSources, fragment_area, fragment_clip, gives_source,
    piece_source, walk_sources,
};

/// A fragment of `node` under the fragment `parent`, with a clip when given.
fn fragment(node: usize, parent: Option<usize>, clip: Option<FragmentRect>) -> LayoutFragment {
    LayoutFragment {
        node_id: node,
        parent,
        fragmentainer: 0,
        rect: FragmentRect {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 100.0,
        },
        fragmentainer_clip: clip,
        fragment_index: 0,
        fragment_count: 1,
        line_start: None,
        line_end: None,
    }
}

fn clip_rect(x: f32, y: f32, width: f32, height: f32) -> Option<FragmentRect> {
    Some(FragmentRect {
        x,
        y,
        width,
        height,
    })
}

#[test]
fn a_fragment_without_a_clip_is_clipped_like_the_fragments_around_it() {
    let plain = fragment(1, None, None);
    assert_eq!(fragment_clip(&plain, (50.0, 70.0), None), None);
    let outer = PaintRect::new(1.0, 2.0, 3.0, 4.0);
    assert_eq!(
        fragment_clip(&plain, (50.0, 70.0), Some(outer)),
        Some(outer)
    );
}

#[test]
fn the_clip_of_a_fragment_moves_with_the_origin_the_fragment_is_placed_at() {
    // The clip is given in the space of the parent fragment, where the
    // fragment sits at (10, 20); it follows the fragment to (50, 70).
    let clipped = fragment(1, None, clip_rect(10.0, 20.0, 60.0, 40.0));
    assert_eq!(
        fragment_clip(&clipped, (50.0, 70.0), None),
        Some(PaintRect::new(50.0, 70.0, 60.0, 40.0))
    );
}

#[test]
fn the_clip_of_a_fragment_is_cut_to_the_clip_around_it() {
    let clipped = fragment(1, None, clip_rect(10.0, 20.0, 60.0, 40.0));
    let outer = PaintRect::new(60.0, 0.0, 100.0, 90.0);
    // (50, 70) to (110, 110) cut to x from 60 and y up to 90.
    assert_eq!(
        fragment_clip(&clipped, (50.0, 70.0), Some(outer)),
        Some(PaintRect::new(60.0, 70.0, 50.0, 20.0))
    );
    // Clips that do not meet leave nothing, not a negative extent.
    let apart = PaintRect::new(500.0, 500.0, 10.0, 10.0);
    assert_eq!(
        fragment_clip(&clipped, (50.0, 70.0), Some(apart)),
        Some(PaintRect::new(500.0, 500.0, 0.0, 0.0))
    );
}

#[test]
fn the_fragments_of_a_document_are_found_by_the_record_they_hang_under() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let div = doc.append_element(Some(body), "div", Some("display:block"));
    let span = doc.append_element(Some(div), "span", None::<&str>);
    let once = doc.append_element(Some(body), "p", Some("display:block"));
    doc.fragment_tree.replace_finalized_fragments(vec![
        fragment(div, None, None),
        fragment(div, None, None),
        fragment(once, None, None),
        fragment(div, Some(0), None),
    ]);
    let index = FragmentIndex::new(&doc);
    assert_eq!(index.placements(None, div), [0, 1]);
    assert_eq!(index.placements(Some(0), div), [3]);
    assert_eq!(index.placements(None, once), [2]);
    assert!(index.placements(Some(0), once).is_empty());
    assert!(index.placements(None, span).is_empty());
    // A node placed more than once repeats with all it contains.
    let repeated: Vec<_> = {
        let mut nodes: Vec<_> = index.repeated.iter().copied().collect();
        nodes.sort_unstable();
        nodes
    };
    assert_eq!(repeated, [div, span]);
}

#[test]
fn a_table_grid_box_is_where_the_page_fragment_of_a_node_is() {
    let mut doc = Document::new();
    let div = doc.append_element(Some(0), "div", None::<&str>);
    let area = PaintRect::new(10.0, 20.0, 30.0, 40.0);
    assert_eq!(fragment_area(&doc.nodes[div], area), area);
    doc.nodes[div].table_grid_box = Some(PaintRect::new(1.0, 2.0, 3.0, 4.0));
    assert_eq!(
        fragment_area(&doc.nodes[div], area),
        PaintRect::new(11.0, 22.0, 3.0, 4.0)
    );
}

#[test]
fn only_elements_and_text_with_something_to_show_give_a_source() {
    let mut doc = Document::new();
    let div = doc.append_element(Some(0), "div", None::<&str>);
    let text = doc.append_text(div, " a ");
    let blank = doc.append_text(div, " \n\t");
    let comment = doc.append_comment(Some(div), "a comment");
    let at = PaintRect::new(1.0, 2.0, 3.0, 4.0);
    assert!(gives_source(&doc.nodes[div], at));
    assert!(gives_source(&doc.nodes[text], at));
    assert!(!gives_source(&doc.nodes[blank], at));
    assert!(!gives_source(&doc.nodes[comment], at));
    // Coordinates that are not finite place nothing.
    for (x, y) in [(f32::NAN, 0.0), (0.0, f32::INFINITY)] {
        assert!(!gives_source(
            &doc.nodes[div],
            PaintRect::new(x, y, 1.0, 1.0)
        ));
    }
}

/// The source of a text node's box, to take the lines of a paragraph.
fn text_source() -> PageFragmentSource {
    PageFragmentSource {
        node_id: NodeId::new(9),
        node_kind: NodeKind::Text,
        is_img: false,
        abs_x: 1.0,
        abs_y: 2.0,
        width: 3.0,
        height: 4.0,
        content_insets: None,
        line_metrics: Some(Vec::new()),
        lines_along_x: false,
        line_start_offset: 0,
        is_repeat: true,
        fragmentainer: 0,
        fragment_clip: Some(PaintRect::new(0.0, 0.0, 5.0, 5.0)),
        overflow_chain: Some(1),
        own_overflow_source: Some(2),
    }
}

fn lines(tops: &[(f32, f32)]) -> IfcTextLines {
    IfcTextLines {
        root: 7,
        width: 90.0,
        lines: tops
            .iter()
            .enumerate()
            .map(|(line, &(top, bottom))| IfcTextLine { line, top, bottom })
            .collect(),
    }
}

/// Paragraph 7, with its content box at (100, 50).
fn paragraphs_at_100_50() -> Paragraphs {
    let mut paragraphs = Paragraphs::default();
    paragraphs.origin.insert(7, (100.0, 50.0));
    paragraphs
}

/// `(x, y, width, height)` of a source.
fn area(source: &PageFragmentSource) -> (f32, f32, f32, f32) {
    (source.abs_x, source.abs_y, source.width, source.height)
}

#[test]
fn horizontal_lines_are_placed_from_the_block_start_edge_of_their_paragraph() {
    let paragraphs = paragraphs_at_100_50();
    let owned = lines(&[(30.0, 40.0), (40.0, 50.0)]);
    let source = paragraphs.line_source(&text_source(), (5.0, 3, 2, &owned), (0.0, 0.0));
    // Offset along the line, then down to the top of the first line.
    assert_eq!(area(&source), (105.0, 80.0, 90.0, 20.0));
    assert!(!source.lines_along_x);
    // Lines are given from the top of the first.
    assert_eq!(source.line_metrics, Some(vec![(0.0, 10.0), (10.0, 20.0)]));
    assert_eq!(source.line_start_offset, 3);
    assert_eq!(source.fragmentainer, 2);
    // All else is the node's.
    assert_eq!(source.node_id, NodeId::new(9));
    assert!(source.is_repeat);
    assert_eq!(source.overflow_chain, Some(1));
    assert_eq!(source.own_overflow_source, Some(2));
}

#[test]
fn right_to_left_vertical_lines_stack_from_the_right_edge_of_their_paragraph() {
    let mut paragraphs = paragraphs_at_100_50();
    paragraphs.vertical.insert(7, (200.0, true));
    let owned = lines(&[(0.0, 10.0), (10.0, 20.0)]);
    let source = paragraphs.line_source(&text_source(), (5.0, 0, 0, &owned), (0.0, 0.0));
    // The lines end 20px in from the right edge of the 200px content box.
    assert_eq!(area(&source), (280.0, 55.0, 20.0, 90.0));
    assert!(source.lines_along_x);
}

#[test]
fn left_to_right_vertical_lines_stack_from_the_left_edge_of_their_paragraph() {
    let mut paragraphs = paragraphs_at_100_50();
    paragraphs.vertical.insert(7, (200.0, false));
    let owned = lines(&[(10.0, 20.0), (20.0, 30.0)]);
    let source = paragraphs.line_source(&text_source(), (5.0, 0, 0, &owned), (0.0, 0.0));
    assert_eq!(area(&source), (110.0, 55.0, 20.0, 90.0));
    assert!(source.lines_along_x);
}

#[test]
fn lines_of_a_paragraph_the_walk_has_not_met_are_placed_from_the_fallback_origin() {
    let paragraphs = Paragraphs::default();
    let source = paragraphs.line_source(&text_source(), (5.0, 0, 0, &lines(&[])), (11.0, 22.0));
    // No lines have no extent along the block axis.
    assert_eq!(area(&source), (16.0, 22.0, 90.0, 0.0));
    assert_eq!(source.line_metrics, Some(Vec::new()));
}

#[test]
fn a_piece_of_an_inline_element_is_placed_from_the_origin_of_its_paragraph() {
    let rect = BoxRect {
        x: 3.0,
        y: 4.0,
        width: 30.0,
        height: f32::NAN,
    };
    let source = piece_source(&text_source(), (10.0, 20.0), &rect, 2);
    // A height that is not finite gives none.
    assert_eq!(area(&source), (13.0, 24.0, 30.0, 0.0));
    assert_eq!(source.fragmentainer, 2);
    assert_eq!(source.node_id, NodeId::new(9));
}

/// `doc` laid out with Ahem, and its sources walked.
fn walked(doc: &mut Document, cascade: &CascadeResult) -> WalkedSources {
    layout_single_page(with_ahem(doc), cascade, page_box_800x600()).expect("layout");
    walk_with(doc, cascade, &PageLayoutControl::default()).expect("walk")
}

fn walk_with(
    doc: &Document,
    cascade: &CascadeResult,
    control: &PageLayoutControl<'_>,
) -> Result<WalkedSources, LayoutError> {
    let mut work = ProjectionWork::new(control, doc.fragment_tree.work_limit(doc.nodes.len()));
    walk_sources(doc, cascade, find_body(doc).expect("body"), &mut work)
}

/// The sources of `node`, in the order they were met.
fn sources_of(walked: &WalkedSources, node: usize) -> Vec<&PageFragmentSource> {
    let id = NodeId::new(node as u64);
    walked
        .nodes
        .iter()
        .filter(|source| source.node_id == id)
        .collect()
}

#[test]
fn a_paragraph_is_a_text_root_and_its_text_a_source_with_the_lines_it_is_on() {
    let mut text = 0;
    let (mut doc, cascade, root) = ahem_paragraph_with("width:200px", |doc, root| {
        text = doc.append_text(root, "aaaa bbbb");
    });
    let walked = walked(&mut doc, &cascade);
    let (x, y, ..) = absolute_rect(&doc, root);
    assert_eq!(
        walked.text_roots,
        [ProjectedTextRoot {
            node: root,
            x,
            y,
            width: 200.0,
            lines_along_x: false,
            is_repeat: false,
            fragmentainer: None,
            fragment_clip: None,
            overflow_chain: None,
        }]
    );
    let [source] = sources_of(&walked, text)[..] else {
        panic!("one source for the one group of lines");
    };
    // The lines are as wide as the paragraph they were broken for.
    assert_eq!(area(source), (x, y, 200.0, 10.0));
    assert_eq!(source.line_metrics, Some(vec![(0.0, 10.0)]));
    assert_eq!(source.line_start_offset, 0);
    assert!(!source.lines_along_x);
    assert!(walked.paragraph_cache.contains_key(&root));
}

#[test]
fn the_text_of_a_paragraph_that_wraps_has_a_line_for_each_row() {
    let mut text = 0;
    let (mut doc, cascade, _) = ahem_paragraph_with("width:50px", |doc, root| {
        text = doc.append_text(root, "aaaa bbbb cccc");
    });
    let walked = walked(&mut doc, &cascade);
    let [source] = sources_of(&walked, text)[..] else {
        panic!("one source for the one group of lines");
    };
    assert_eq!(source.height, 30.0);
    assert_eq!(
        source.line_metrics,
        Some(vec![(0.0, 10.0), (10.0, 20.0), (20.0, 30.0)])
    );
}

#[test]
fn an_inline_element_is_where_its_pieces_are_on_the_lines() {
    let (mut span, mut inner) = (0, 0);
    let (mut doc, cascade, root) = ahem_paragraph_with("width:200px", |doc, root| {
        doc.append_text(root, "aa ");
        span = doc.append_element(Some(root), "span", Some("display:inline"));
        inner = doc.append_text(span, "bbbb");
    });
    let walked = walked(&mut doc, &cascade);
    let (x, y, ..) = absolute_rect(&doc, root);
    let [source] = sources_of(&walked, span)[..] else {
        panic!("one source for the one piece");
    };
    assert_eq!(area(source), (x + 30.0, y, 40.0, 10.0));
    // The span is no paragraph: its text is on the lines of the paragraph.
    assert!(walked.text_roots.iter().all(|root| root.node != span));
    let [text] = sources_of(&walked, inner)[..] else {
        panic!("one source for the text");
    };
    assert_eq!(area(text), (x, y, 200.0, 10.0));
}

#[test]
fn what_is_not_rendered_gives_no_source() {
    let (mut blank, mut comment, mut hidden, mut hidden_text, mut shown) = (0, 0, 0, 0, 0);
    let (mut doc, cascade, _) = ahem_paragraph_with("width:200px", |doc, root| {
        blank = doc.append_text(root, " ");
        comment = doc.append_comment(Some(root), "a comment");
        hidden = doc.append_element(Some(root), "span", Some("display:none"));
        hidden_text = doc.append_text(hidden, "hidden");
        shown = doc.append_text(root, "shown");
    });
    let walked = walked(&mut doc, &cascade);
    for node in [blank, comment, hidden, hidden_text] {
        assert!(sources_of(&walked, node).is_empty(), "node {node}");
    }
    assert_eq!(sources_of(&walked, shown).len(), 1);
}

#[test]
fn a_fixed_subtree_is_marked_as_repeating_on_every_page() {
    let (mut fixed, mut inner, mut flowing) = (0, 0, 0);
    let (mut doc, cascade, root) = ahem_paragraph_with("width:200px", |doc, root| {
        fixed = doc.append_element(
            Some(root),
            "div",
            Some("display:block;position:fixed;top:0;left:0"),
        );
        inner = doc.append_text(fixed, "fixed");
        flowing = doc.append_text(root, "flowing");
    });
    let walked = walked(&mut doc, &cascade);
    for node in [fixed, inner] {
        assert!(sources_of(&walked, node)[0].is_repeat, "node {node}");
    }
    for node in [root, flowing] {
        assert!(!sources_of(&walked, node)[0].is_repeat, "node {node}");
    }
}

#[test]
fn the_sources_are_in_node_order_though_a_node_is_placed_more_than_once() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let wrapper = doc.append_element(Some(body), "div", Some("display:block"));
    for _ in 0..2 {
        doc.append_element(Some(wrapper), "div", Some("display:block"));
    }
    doc.fragment_tree.replace_finalized_fragments(vec![
        fragment(wrapper, None, None),
        fragment(wrapper, None, None),
    ]);
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    let walked = walk_with(&doc, &cascade, &PageLayoutControl::default()).expect("walk");
    // The walk meets the wrapper and the boxes it holds once for each of its
    // two placements, and hands the sources over in node order.
    assert_eq!(sources_of(&walked, wrapper).len(), 2);
    let ids: Vec<_> = walked.nodes.iter().map(|source| source.node_id).collect();
    assert!(ids.windows(2).all(|pair| pair[0] <= pair[1]), "{ids:?}");
}

#[test]
fn a_paragraph_that_stacks_its_lines_along_x_says_so() {
    let mut text = 0;
    let (mut doc, cascade, root) = ahem_paragraph_with(
        "width:100px;height:200px;writing-mode:vertical-rl",
        |doc, root| {
            text = doc.append_text(root, "aaaa");
        },
    );
    let walked = walked(&mut doc, &cascade);
    let [text_root] = walked.text_roots[..] else {
        panic!("one paragraph");
    };
    assert_eq!(text_root.node, root);
    assert!(text_root.lines_along_x);
    assert!(sources_of(&walked, text)[0].lines_along_x);
}

#[test]
fn the_marker_of_a_list_item_is_a_text_root_of_its_own() {
    let (mut doc, cascade, root) = ahem_paragraph_with(
        "width:200px;display:list-item;list-style-type:disc",
        |doc, root| {
            doc.append_text(root, "item");
        },
    );
    let walked = walked(&mut doc, &cascade);
    let (x, y, ..) = absolute_rect(&doc, root);
    let marker =
        crate::generated_content::generated_node_id(root, raikiri_style::PseudoElem::Marker);
    let roots: Vec<_> = walked
        .text_roots
        .iter()
        .map(|root| (root.node, root.x, root.y, root.width))
        .collect();
    assert_eq!(roots, [(marker, x, y, 0.0), (root, x, y, 200.0)]);
}

#[test]
fn a_walk_that_is_aborted_stops_with_the_error() {
    let (mut doc, cascade, _) = ahem_paragraph_with("width:200px", |doc, root| {
        doc.append_text(root, "text");
    });
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    let abort = || true;
    let control = PageLayoutControl::default().with_abort_check(&abort);
    assert!(matches!(
        walk_with(&doc, &cascade, &control),
        Err(LayoutError::Aborted)
    ));
}

#[test]
fn the_text_of_a_table_is_a_paragraph_of_the_anonymous_cell_that_holds_it() {
    let (mut doc, cascade, table) = ahem_table_with_only_text("", "aaaa bbbb");
    let walked = walked(&mut doc, &cascade);
    let (cell, cell_node) = doc.anonymous_table_cells(table).next().expect("a cell");
    let (x, y, ..) = absolute_rect(&doc, table);
    let location = cell_node.layout.location;
    let [root] = walked
        .text_roots
        .iter()
        .filter(|root| root.node == cell)
        .collect::<Vec<_>>()[..]
    else {
        panic!("one text root for the cell");
    };
    // The cell is placed in the table, which gives the box of its own text.
    assert_eq!((root.x, root.y), (x + location.x, y + location.y));
    assert!(walked.paragraph_cache.contains_key(&cell));
}
